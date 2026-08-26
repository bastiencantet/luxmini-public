//! `luxmini` — a tiny command-line client that drives the Mac front LED through
//! `LuxMini`'s local control API (see `crate::api`). It is the scripting entry
//! point: Apple Shortcuts, `AppleScript` (`do shell script`), Home Assistant
//! `command_line`, and plain shell scripts all shell out to it.
//!
//! Requires the `LuxMini` app to be running with the local API enabled
//! (Settings › General). Configuration via environment:
//!   `LUXMINI_API_PORT`  — API port (default 4470)
//!   `LUXMINI_API_TOKEN` — bearer token, if one is set in the app
//!
//! Dependency-free: a minimal blocking `HTTP/1.1` client on `std::net`.

use std::fmt::Write as _;
use std::io::{Read, Write as _};
use std::net::TcpStream;
use std::process::ExitCode;
use std::time::Duration;

const USAGE: &str = "\
luxmini — control the Mac front LED via LuxMini's local API

usage:
  luxmini get                  show the current state
  luxmini on | off             turn the LED on / off
  luxmini brightness <0-255>   set brightness
  luxmini effect <name>        blink | blinkfast | pulse | sos | strobe | none
  luxmini stop                 stop any running effect

env: LUXMINI_API_PORT (default 4470), LUXMINI_API_TOKEN
Requires LuxMini running with the local API enabled (Settings > General).";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let body = match args.first().map(String::as_str) {
        Some("get" | "status") => None,
        Some("on") => Some(r#"{"on":true}"#.to_string()),
        Some("off") => Some(r#"{"on":false}"#.to_string()),
        Some("stop") => Some(r#"{"effect":"none"}"#.to_string()),
        Some("brightness") => {
            let Some(v) = args.get(1).and_then(|s| s.parse::<u16>().ok()) else {
                eprintln!("usage: luxmini brightness <0-255>");
                return ExitCode::from(2);
            };
            Some(format!(r#"{{"brightness":{}}}"#, v.min(255)))
        }
        Some("effect") => {
            let Some(name) = args.get(1) else {
                eprintln!("usage: luxmini effect <blink|blinkfast|pulse|sos|strobe|none>");
                return ExitCode::from(2);
            };
            Some(format!(r#"{{"effect":"{name}"}}"#))
        }
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    match request(if body.is_some() { "POST" } else { "GET" }, body.as_deref()) {
        Ok(response) => {
            println!("{response}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!(
                "luxmini: {e}\n(is LuxMini running with the local API enabled? — Settings › General)"
            );
            ExitCode::FAILURE
        }
    }
}

/// Send one request to `/led` on the local API and return the response body.
fn request(method: &str, body: Option<&str>) -> std::io::Result<String> {
    let port: u16 = std::env::var("LUXMINI_API_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(4470);
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;

    let body = body.unwrap_or("");
    let mut request = format!("{method} /led HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    if let Ok(token) = std::env::var("LUXMINI_API_TOKEN") {
        if !token.is_empty() {
            if token.contains(['\r', '\n']) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "LUXMINI_API_TOKEN contains a newline",
                ));
            }
            let _ = write!(request, "Authorization: Bearer {token}\r\n");
        }
    }
    if !body.is_empty() {
        let _ = write!(
            request,
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        );
    }
    request.push_str("\r\n");
    request.push_str(body);

    stream.write_all(request.as_bytes())?;
    stream.flush()?;

    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    parse_response(&response)
}

fn parse_response(response: &str) -> std::io::Result<String> {
    let (headers, body) = response.split_once("\r\n\r\n").ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid HTTP response")
    })?;
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid HTTP status")
        })?;
    if !(200..300).contains(&status) {
        return Err(std::io::Error::other(format!(
            "HTTP {status}: {}",
            body.trim()
        )));
    }
    Ok(body.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::parse_response;

    #[test]
    fn returns_the_body_for_successful_responses() {
        let parsed = parse_response("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}");
        assert!(matches!(parsed.as_deref(), Ok("{}")));
    }

    #[test]
    fn turns_http_failures_into_cli_errors() {
        let error = parse_response(
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: 24\r\n\r\n{\"error\":\"unauthorized\"}",
        )
        .err();
        assert!(error.is_some_and(|value| value.to_string().contains("HTTP 401")));
    }

    #[test]
    fn rejects_malformed_http() {
        assert!(parse_response("not http").is_err());
        assert!(parse_response("HTTP/1.1 nope\r\n\r\n{}").is_err());
    }
}
