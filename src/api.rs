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
//! - `GET /healthz` → `{"status":"ok"}`
//! - `GET /led` → `{"on":bool,"brightness":0..255,"max":255}`
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
        ("GET", "/healthz") => respond(&mut stream, 200, r#"{"status":"ok"}"#),
        ("GET", "/led") => respond(&mut stream, 200, &led_json()),
        ("POST", "/led") => {
            if apply(&body) {
                respond(&mut stream, 200, &led_json())
            } else {
                respond(&mut stream, 400, r#"{"error":"invalid command"}"#)
            }
        }
        _ => respond(&mut stream, 404, r#"{"error":"not found"}"#),
    }
}

fn led_json() -> String {
    let (on, brightness) = led::read_state();
    format!(r#"{{"on":{on},"brightness":{brightness},"max":255}}"#)
}

/// Apply a `POST /led` body. Minimal, dependency-free extraction for a fixed,
/// tiny shape — not a general JSON parser. `effect` takes precedence, then
/// `on`, then `brightness`.
fn apply(body: &str) -> bool {
    if let Some(effect) = json_str(body, "effect") {
        let parsed = match effect.as_str() {
            "blink" => Some(Effect::Blink),
            "blinkfast" | "blink_fast" => Some(Effect::BlinkFast),
            "pulse" => Some(Effect::Pulse),
            "sos" => Some(Effect::Sos),
            "strobe" => Some(Effect::Strobe),
            "none" | "off" | "clear" | "stop" => None,
            _ => return false,
        };
        led::with_state(|s| match parsed {
            Some(effect) => s.start_effect(effect),
            None => s.clear_effect(),
        });
        return true;
    }
    let mut applied = false;
    if let Some(on) = json_bool(body, "on") {
        led::with_state(|s| s.set_on(on));
        applied = true;
    }
    if let Some(value) = json_u8(body, "brightness") {
        led::with_state(|s| s.set_brightness(value));
        applied = true;
    }
    applied
}

/// Slice of `body` immediately after `"<key>":`, whitespace-trimmed. Uses
/// `str::get` throughout so a malformed body can never panic on an index.
fn json_after<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\"");
    let pos = body.find(&needle)?;
    let rest = body.get(pos + needle.len()..)?.trim_start();
    Some(rest.strip_prefix(':')?.trim_start())
}

fn json_bool(body: &str, key: &str) -> Option<bool> {
    let rest = json_after(body, key)?;
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn json_u8(body: &str, key: &str) -> Option<u8> {
    let rest = json_after(body, key)?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits
        .parse::<u16>()
        .ok()
        .and_then(|n| u8::try_from(n).ok())
}

fn json_str(body: &str, key: &str) -> Option<String> {
    let inner = json_after(body, key)?.strip_prefix('"')?;
    let end = inner.find('"')?;
    Some(inner.get(..end)?.to_ascii_lowercase())
}

fn respond(stream: &mut TcpStream, status: u16, body: &str) -> std::io::Result<()> {
    let reason = match status {
        400 => "Bad Request",
        401 => "Unauthorized",
        413 => "Content Too Large",
        404 => "Not Found",
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
    use super::{apply, json_bool, json_str, json_u8};

    #[test]
    fn parses_bool() {
        assert_eq!(json_bool(r#"{"on":true}"#, "on"), Some(true));
        assert_eq!(json_bool(r#"{ "on" : false }"#, "on"), Some(false));
        assert_eq!(json_bool(r#"{"on":1}"#, "on"), None);
        assert_eq!(json_bool("{}", "on"), None);
    }

    #[test]
    fn parses_brightness_and_rejects_out_of_range_values() {
        assert_eq!(json_u8(r#"{"brightness":128}"#, "brightness"), Some(128));
        assert_eq!(json_u8(r#"{"brightness":999}"#, "brightness"), None);
        assert_eq!(json_u8(r#"{"brightness":0}"#, "brightness"), Some(0));
        assert_eq!(json_u8("{}", "brightness"), None);
    }

    #[test]
    fn parses_effect_string_lowercased() {
        assert_eq!(
            json_str(r#"{"effect":"Blink"}"#, "effect"),
            Some("blink".into())
        );
        assert_eq!(
            json_str(r#"{"effect":"sos"}"#, "effect"),
            Some("sos".into())
        );
        assert_eq!(json_str("{}", "effect"), None);
    }

    #[test]
    fn malformed_body_never_panics() {
        assert_eq!(json_str(r#"{"effect":"#, "effect"), None);
        assert_eq!(json_u8(r#"{"brightness":"#, "brightness"), None);
        assert_eq!(json_bool(r#"{"on"#, "on"), None);
    }

    #[test]
    fn rejects_empty_and_unknown_commands() {
        assert!(!apply("{}"));
        assert!(!apply(r#"{"effect":"unknown"}"#));
        assert!(!apply(r#"{"brightness":999}"#));
    }

    #[test]
    fn accepts_supported_commands_without_global_state() {
        assert!(apply(r#"{"on":true}"#));
        assert!(apply(r#"{"brightness":128}"#));
        assert!(apply(r#"{"effect":"none"}"#));
    }
}
