//! Optional local HTTP control API — opt-in, localhost-only.
//!
//! Lets local tools (Home Assistant, Apple Shortcuts, `AppleScript`, shell
//! scripts) read and drive the front LED with no cloud, account, or tracking.
//! Disabled by default; turned on via the `api.enabled` preference. Binds
//! `127.0.0.1` only and, when `api.token` is set, requires an
//! `Authorization: Bearer <token>` header.
//!
//! Deliberately dependency-free: a tiny blocking `HTTP/1.1` server on a
//! background thread, matching the app's no-async, minimal-deps style. It is not
//! a general web server — it handles a few small `JSON` routes, one request per
//! connection, then closes.
//!
//! Routes:
//! - `GET /healthz` → status 200 when the helper is ready, 503 otherwise.
//! - `GET /led` → state and helper readiness.
//! - `POST /led` → apply `{"on":bool}` / `{"brightness":0..255}` /
//!   `{"effect":"blink|blinkfast|pulse|sos|strobe|none"}`, then return the state.

use crate::led::{self, Effect};
use crate::preferences;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Hard cap on a request body — this API only ever receives tiny JSON.
const MAX_BODY: usize = 4 * 1024;
const MAX_HEADERS: usize = 64;
const IO_TIMEOUT: Duration = Duration::from_secs(2);

static RUNNING: AtomicBool = AtomicBool::new(false);
static STOP: AtomicBool = AtomicBool::new(false);

/// Start the local API on a background thread when `api.enabled` is set.
/// No-op otherwise (the default), so the app ships with no open port.
pub fn maybe_start() {
    if !preferences::api_enabled() {
        stop();
        return;
    }
    if RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    STOP.store(false, Ordering::Release);
    let port = preferences::api_port();
    if std::thread::Builder::new()
        .name("luxmini-api".into())
        .spawn(move || {
            serve(port);
            RUNNING.store(false, Ordering::Release);
        })
        .is_err()
    {
        RUNNING.store(false, Ordering::Release);
    }
}

/// Ask the background server to release its listener. The listener is
/// non-blocking, so shutdown completes within one polling interval.
pub fn stop() {
    STOP.store(true, Ordering::Release);
}

fn serve(port: u16) {
    // Localhost only — the LED control must never be exposed on the network.
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("local API: cannot bind 127.0.0.1:{port}: {e}");
            return;
        }
    };
    if let Err(e) = listener.set_nonblocking(true) {
        eprintln!("local API: cannot configure listener: {e}");
        return;
    }
    eprintln!("local API listening on http://127.0.0.1:{port}");
    while !STOP.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => {
                // One request per connection; a per-connection error just drops it.
                let _ = handle(stream);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                eprintln!("local API: accept failed: {e}");
                break;
            }
        }
    }
    eprintln!("local API stopped");
}

fn handle(mut stream: TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let mut reader = BufReader::new(stream.try_clone()?);

    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();

    let mut content_length = 0usize;
    let mut auth: Option<String> = None;
    for _ in 0..MAX_HEADERS {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            match key.trim().to_ascii_lowercase().as_str() {
                "content-length" => match value.trim().parse() {
                    Ok(length) => content_length = length,
                    Err(_) => {
                        return respond(&mut stream, 400, r#"{"error":"bad content length"}"#)
                    }
                },
                "authorization" => auth = Some(value.trim().to_string()),
                _ => {}
            }
        }
    }

    // Auth gate. When a token is configured it is mandatory; otherwise the
    // localhost binding is the trust boundary.
    if let Some(expected) = preferences::api_token() {
        let presented = auth.as_deref().and_then(|a| a.strip_prefix("Bearer "));
        if presented != Some(expected.as_str()) {
            return respond(&mut stream, 401, r#"{"error":"unauthorized"}"#);
        }
    }

    if content_length > MAX_BODY {
        return respond(&mut stream, 413, r#"{"error":"body too large"}"#);
    }

    let mut body = String::new();
    if content_length > 0 {
        let mut buf = vec![0u8; content_length];
        reader.read_exact(&mut buf)?;
        body = String::from_utf8_lossy(&buf).into_owned();
    }

    match (method.as_str(), path.as_str()) {
        ("GET", "/healthz") => {
            if led::helper_available() {
                respond(&mut stream, 200, r#"{"status":"ok"}"#)
            } else {
                respond(
                    &mut stream,
                    503,
                    r#"{"status":"degraded","helper":"unavailable"}"#,
                )
            }
        }
        ("GET", "/led") => respond(&mut stream, 200, &led_json()),
        ("POST", "/led") => match apply(&body) {
            ApplyResult::Applied => respond(&mut stream, 200, &led_json()),
            ApplyResult::Invalid => respond(&mut stream, 400, r#"{"error":"invalid command"}"#),
            ApplyResult::Unavailable => {
                respond(&mut stream, 503, r#"{"error":"led helper unavailable"}"#)
            }
        },
        _ => respond(&mut stream, 404, r#"{"error":"not found"}"#),
    }
}

fn led_json() -> String {
    let status = led::read_status();
    let on = status.is_on;
    let brightness = status.brightness;
    let helper_ready = status.helper_ready;
    format!(r#"{{"on":{on},"brightness":{brightness},"max":255,"helper_ready":{helper_ready}}}"#)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LedRequest {
    Effect(Option<Effect>),
    On(bool),
    Brightness(u8),
    OnAndBrightness(bool, u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApplyResult {
    Applied,
    Invalid,
    Unavailable,
}

/// Parse a small `POST /led` command without touching the global LED state.
/// An effect takes precedence; when both on and brightness are supplied, both
/// commands are applied in that order for compatibility with existing clients.
fn parse_request(body: &str) -> Option<LedRequest> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let object = value.as_object()?;
    if let Some(effect) = object.get("effect") {
        let effect = effect.as_str()?.to_ascii_lowercase();
        let parsed = match effect.as_str() {
            "blink" => Some(Effect::Blink),
            "blinkfast" | "blink_fast" => Some(Effect::BlinkFast),
            "pulse" => Some(Effect::Pulse),
            "sos" => Some(Effect::Sos),
            "strobe" => Some(Effect::Strobe),
            "none" | "off" | "clear" | "stop" => None,
            _ => return None,
        };
        return Some(LedRequest::Effect(parsed));
    }
    let on = object
        .get("on")
        .map_or(Some(None), |value| value.as_bool().map(Some))?;
    let brightness = object.get("brightness").map_or(Some(None), |value| {
        value.as_u64().and_then(|n| u8::try_from(n).ok()).map(Some)
    })?;
    match (on, brightness) {
        (Some(on), Some(brightness)) => Some(LedRequest::OnAndBrightness(on, brightness)),
        (Some(on), None) => Some(LedRequest::On(on)),
        (None, Some(brightness)) => Some(LedRequest::Brightness(brightness)),
        (None, None) => None,
    }
}

fn apply(body: &str) -> ApplyResult {
    let Some(command) = parse_request(body) else {
        return ApplyResult::Invalid;
    };
    let succeeded = led::with_state(|state| match command {
        LedRequest::Effect(Some(effect)) => {
            state.start_effect(effect);
            !state.control_failed()
        }
        LedRequest::Effect(None) => {
            state.clear_effect();
            !state.control_failed()
        }
        LedRequest::On(on) => {
            state.set_on(on);
            !state.control_failed()
        }
        LedRequest::Brightness(value) => {
            state.set_brightness(value);
            !state.control_failed()
        }
        LedRequest::OnAndBrightness(on, value) => {
            state.set_on(on);
            let first_succeeded = !state.control_failed();
            state.set_brightness(value);
            first_succeeded && !state.control_failed()
        }
    });
    if succeeded == Some(true) {
        ApplyResult::Applied
    } else {
        ApplyResult::Unavailable
    }
}

fn respond(stream: &mut TcpStream, status: u16, body: &str) -> std::io::Result<()> {
    let reason = match status {
        400 => "Bad Request",
        401 => "Unauthorized",
        413 => "Content Too Large",
        404 => "Not Found",
        503 => "Service Unavailable",
        _ => "OK",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::{apply, parse_request, ApplyResult, LedRequest};

    #[test]
    fn parses_brightness_and_rejects_out_of_range_values() {
        assert_eq!(
            parse_request(r#"{"brightness":128}"#),
            Some(LedRequest::Brightness(128))
        );
        assert_eq!(parse_request(r#"{"brightness":999}"#), None);
        assert_eq!(
            parse_request(r#"{"brightness":0}"#),
            Some(LedRequest::Brightness(0))
        );
    }

    #[test]
    fn parses_effect_string_lowercased() {
        assert_eq!(
            parse_request(r#"{"effect":"Blink"}"#),
            Some(LedRequest::Effect(Some(crate::led::Effect::Blink)))
        );
        assert_eq!(
            parse_request(r#"{"effect":"sos"}"#),
            Some(LedRequest::Effect(Some(crate::led::Effect::Sos)))
        );
        assert_eq!(parse_request("{}"), None);
    }

    #[test]
    fn malformed_body_never_panics() {
        assert_eq!(parse_request(r#"{"effect":"#), None);
        assert_eq!(parse_request(r#"{"brightness":"#), None);
        assert_eq!(parse_request(r#"{"on"#), None);
        assert_eq!(parse_request(r#"{"brightness":128garbage}"#), None);
        assert_eq!(parse_request(r#"{"note":"on":true}"#), None);
        assert_eq!(parse_request(r#"{"on":"false","brightness":40}"#), None);
    }

    #[test]
    fn rejects_empty_and_unknown_commands() {
        assert_eq!(apply("{}"), ApplyResult::Invalid);
        assert_eq!(apply(r#"{"effect":"unknown"}"#), ApplyResult::Invalid);
        assert_eq!(apply(r#"{"brightness":999}"#), ApplyResult::Invalid);
    }

    #[test]
    fn parses_supported_commands_without_global_state() {
        assert_eq!(parse_request(r#"{"on":true}"#), Some(LedRequest::On(true)));
        assert_eq!(
            parse_request(r#"{"brightness":128}"#),
            Some(LedRequest::Brightness(128))
        );
        assert_eq!(
            parse_request(r#"{"effect":"none"}"#),
            Some(LedRequest::Effect(None))
        );
    }

    #[test]
    fn valid_command_without_led_state_is_unavailable() {
        assert_eq!(apply(r#"{"on":true}"#), ApplyResult::Unavailable);
    }
}
