//! Optional, aggregate-only product diagnostics.
//!
//! Diagnostics are disabled by default. When enabled, `LuxMini` sends a bounded
//! event name, outcome, Mac model, and macOS major version. It never creates an
//! installation identifier and never sends a serial number, account, location,
//! SMC value, LED brightness, schedule, preset, or local API request.

use serde::{Deserialize, Serialize};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::preferences;

const PROFILE_TOKEN_URL: &str = "https://api.luxmini.bastiencantet.com/api/v1/profile-token";
const TELEMETRY_URL: &str = "https://api.luxmini.bastiencantet.com/api/v1/telemetry";
const RETENTION_SECONDS: i64 = 7 * 24 * 60 * 60;

#[derive(Serialize)]
struct TokenRequest<'a> {
    model_id: &'a str,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Serialize)]
struct TelemetryRequest<'a> {
    event: &'a str,
    model_id: &'a str,
    outcome: &'a str,
    macos_major: u8,
}

/// Record the local first-seen time and emit the one-time launch and retention
/// milestones when the user has explicitly enabled anonymous diagnostics.
pub fn note_launch(model: &str) {
    let now = unix_timestamp();
    preferences::ensure_telemetry_first_seen(now);
    if !preferences::diagnostics_enabled() {
        return;
    }
    emit_once("app_first_launch", "success", model);
    if now.saturating_sub(preferences::telemetry_first_seen_at()) >= RETENTION_SECONDS {
        emit_once("retained_7d", "active", model);
    }
}

/// Send one aggregate event at most once for this installation. The local
/// marker prevents retries from inflating milestone metrics, but is never sent.
pub fn emit_once(event: &'static str, outcome: &'static str, model: &str) {
    if !preferences::diagnostics_enabled() || preferences::telemetry_event_sent(event) {
        return;
    }
    let model = model.to_string();
    let _ = std::thread::Builder::new()
        .name("luxmini-diagnostics".into())
        .spawn(move || {
            if send(event, outcome, &model) {
                preferences::mark_telemetry_event_sent(event);
            }
        });
}

/// Send a non-milestone aggregate event. Callers must use only the bounded
/// event and outcome vocabulary accepted by the profile service.
pub fn emit(event: &'static str, outcome: &'static str, model: &str) {
    if !preferences::diagnostics_enabled() {
        return;
    }
    let model = model.to_string();
    let _ = std::thread::Builder::new()
        .name("luxmini-diagnostics".into())
        .spawn(move || {
            let _ = send(event, outcome, &model);
        });
}

fn send(event: &str, outcome: &str, model: &str) -> bool {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(3))
        .timeout_read(Duration::from_secs(5))
        .timeout_write(Duration::from_secs(5))
        .build();
    let Ok(response) = agent
        .post(PROFILE_TOKEN_URL)
        .send_json(TokenRequest { model_id: model })
    else {
        return false;
    };
    let Ok(token) = response.into_json::<TokenResponse>() else {
        return false;
    };
    agent
        .post(TELEMETRY_URL)
        .set("Authorization", &format!("Bearer {}", token.access_token))
        .send_json(TelemetryRequest {
            event,
            model_id: model,
            outcome,
            macos_major: macos_major(),
        })
        .is_ok()
}

fn macos_major() -> u8 {
    Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|version| version.trim().split('.').next()?.parse().ok())
        .unwrap_or(0)
}

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::RETENTION_SECONDS;

    #[test]
    fn retention_window_is_seven_days() {
        assert_eq!(RETENTION_SECONDS, 604_800);
    }
}
