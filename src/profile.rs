//! Device profile — *how* to address the front LED on this Mac model.
//!
//! The SMC mechanism (write a PWM value to a 4-char key) lives in the source; the
//! per-model specifics — which key, what byte layout, the max value — are **data**,
//! not code. Profiles are fetched for the current hardware model from the
//! `LuxMini` API, validated, and cached locally. No profile catalog or shared
//! credential is baked into the executable.
//!
//! Profile file (plain text, one active line `KEY FORMAT MAX`):
//!   ~/Library/Application Support/LuxMini/profile
//! e.g.   `XXXX vv 255`     (vv = write [v,v];  v0 = write [v,0])
//! See `profile.example` in the repo for the format (no real key shipped).

use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

const PROFILE_TOKEN_URL: &str = "https://api.luxmini.bastiencantet.com/api/v1/profile-token";
const PROFILE_URL: &str = "https://api.luxmini.bastiencantet.com/api/v1/profile";
const CANDIDATE_PROFILE_URL: &str =
    "https://api.luxmini.bastiencantet.com/api/v1/candidate-profile";
const PROFILE_VALIDATION_URL: &str =
    "https://api.luxmini.bastiencantet.com/api/v1/profile-validation";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceProfile {
    /// 4-character SMC key for the front LED.
    pub key: String,
    /// true  → write the value as [v, v]  (most models)
    /// false → write the value as [v, 0]  (older Intel case-LED)
    pub replicate: bool,
    /// device max brightness (the logical 0..255 is scaled to this).
    pub max: u8,
}

#[derive(Serialize)]
struct TokenRequest<'a> {
    model_id: &'a str,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Deserialize)]
struct ProfileResponse {
    model_id: String,
    key: String,
    format: String,
    max: u8,
}

#[derive(Serialize)]
struct ValidationRequest<'a> {
    model_id: &'a str,
    outcome: &'a str,
}

fn profile_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Application Support/LuxMini/profile"))
}

impl DeviceProfile {
    /// Load a cached profile or fetch and cache the profile matching this Mac.
    pub fn load() -> Option<Self> {
        let path = profile_path()?;
        if let Some(profile) = Self::load_file(&path) {
            return Some(profile);
        }
        let model = crate::compat::get_mac_model();
        let profile = Self::fetch(&model)?;
        if let Err(error) = profile.cache(&path) {
            eprintln!("cannot cache device profile: {error}");
        }
        Some(profile)
    }

    fn load_file(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        let line = text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with('#'))?;
        Self::parse_line(line)
    }

    fn agent() -> ureq::Agent {
        ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(3))
            .timeout_read(Duration::from_secs(5))
            .timeout_write(Duration::from_secs(5))
            .build()
    }

    fn token(agent: &ureq::Agent, model: &str) -> Option<String> {
        let token: TokenResponse = agent
            .post(PROFILE_TOKEN_URL)
            .send_json(TokenRequest { model_id: model })
            .ok()?
            .into_json()
            .ok()?;
        Some(token.access_token)
    }

    fn fetch_from(model: &str, url: &str) -> Option<Self> {
        let agent = Self::agent();
        let token = Self::token(&agent, model)?;
        let response: ProfileResponse = agent
            .get(url)
            .query("model_id", model)
            .set("Authorization", &format!("Bearer {token}"))
            .call()
            .ok()?
            .into_json()
            .ok()?;
        Self::from_response(&response, model)
    }

    fn fetch(model: &str) -> Option<Self> {
        Self::fetch_from(model, PROFILE_URL)
    }

    /// Fetch a pending profile for the explicit visual hardware test. Candidate
    /// profiles are never cached implicitly.
    pub fn fetch_candidate(model: &str) -> Option<Self> {
        Self::fetch_from(model, CANDIDATE_PROFILE_URL)
    }

    /// Return a locally cached profile without contacting the API.
    pub fn load_cached() -> Option<Self> {
        Self::load_file(&profile_path()?)
    }

    /// Cache a candidate only after the user has visually confirmed the fade.
    pub fn cache_validated(&self) -> std::io::Result<()> {
        let path = profile_path().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "home directory unavailable")
        })?;
        self.cache(&path)
    }

    /// Send an anonymous validation outcome. This is best-effort and contains
    /// only the hardware model and the answer; no serial or installation ID.
    pub fn report_validation(model: &str, outcome: &str) -> bool {
        if !matches!(outcome, "yes" | "no" | "technical_error") {
            return false;
        }
        let agent = Self::agent();
        let Some(token) = Self::token(&agent, model) else {
            return false;
        };
        agent
            .post(PROFILE_VALIDATION_URL)
            .set("Authorization", &format!("Bearer {token}"))
            .send_json(ValidationRequest {
                model_id: model,
                outcome,
            })
            .is_ok()
    }

    fn from_response(response: &ProfileResponse, expected_model: &str) -> Option<Self> {
        if response.model_id != expected_model
            || !matches!(response.format.as_str(), "vv" | "v0")
            || response.max == 0
        {
            return None;
        }
        let line = format!("{} {} {}", response.key, response.format, response.max);
        Self::parse_line(&line)
    }

    fn cache(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let format = if self.replicate { "vv" } else { "v0" };
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        writeln!(file, "{} {format} {}", self.key, self.max)
    }

    /// Parse a single `KEY FORMAT MAX` profile line. Pure (no filesystem), so it
    /// is unit-testable. `FORMAT` defaults to `vv`, `MAX` to 255.
    fn parse_line(line: &str) -> Option<Self> {
        let mut it = line.split_whitespace();
        let key = it.next()?.to_string();
        // SMC keys are exactly 4 ASCII characters.
        if key.len() != 4 || !key.is_ascii() {
            return None;
        }
        let replicate = it.next() != Some("v0");
        let max = it.next().and_then(|m| m.parse().ok()).unwrap_or(255u8);
        Some(Self {
            key,
            replicate,
            max,
        })
    }

    /// The two SMC data bytes for a logical brightness 0..=255.
    #[allow(clippy::cast_possible_truncation)] // const fn keeps `as u16`; final narrow is 0..=255
    pub const fn bytes(&self, value: u8) -> [u8; 2] {
        let scaled = ((value as u16 * self.max as u16) / 255) as u8;
        if self.replicate {
            [scaled, scaled]
        } else {
            [scaled, 0]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DeviceProfile, ProfileResponse};

    #[test]
    fn parses_full_line() {
        assert_eq!(
            DeviceProfile::parse_line("ABCD vv 200"),
            Some(DeviceProfile {
                key: "ABCD".into(),
                replicate: true,
                max: 200,
            })
        );
    }

    #[test]
    fn v0_disables_replicate() {
        assert!(matches!(
            DeviceProfile::parse_line("ABCD v0 255"),
            Some(DeviceProfile {
                replicate: false,
                ..
            })
        ));
        // any format marker other than "v0" keeps replicate on
        assert!(matches!(
            DeviceProfile::parse_line("ABCD vv 255"),
            Some(DeviceProfile {
                replicate: true,
                ..
            })
        ));
    }

    #[test]
    fn defaults_format_and_max() {
        assert_eq!(
            DeviceProfile::parse_line("ABCD"),
            Some(DeviceProfile {
                key: "ABCD".into(),
                replicate: true,
                max: 255,
            })
        );
    }

    #[test]
    fn rejects_bad_keys() {
        assert!(DeviceProfile::parse_line("ABC").is_none()); // too short
        assert!(DeviceProfile::parse_line("ABCDE").is_none()); // too long
        assert!(DeviceProfile::parse_line("ÀBCD").is_none()); // non-ASCII (2-byte À)
        assert!(DeviceProfile::parse_line("").is_none());
    }

    #[test]
    fn max_falls_back_on_garbage() {
        assert!(matches!(
            DeviceProfile::parse_line("ABCD vv notanum"),
            Some(DeviceProfile { max: 255, .. })
        ));
    }

    #[test]
    fn parser_is_tolerant() {
        // 4-char ASCII keys are accepted regardless of case.
        assert_eq!(
            DeviceProfile::parse_line("abcd vv 255").map(|p| p.key),
            Some("abcd".to_string())
        );
        // Tokens past MAX are ignored, not rejected.
        assert!(matches!(
            DeviceProfile::parse_line("ABCD vv 100 garbage"),
            Some(DeviceProfile { max: 100, .. })
        ));
    }

    #[test]
    fn bytes_scaling() {
        let full = DeviceProfile {
            key: "ABCD".into(),
            replicate: true,
            max: 255,
        };
        assert_eq!(full.bytes(0), [0, 0]);
        assert_eq!(full.bytes(128), [128, 128]);
        assert_eq!(full.bytes(255), [255, 255]);

        let capped = DeviceProfile {
            key: "ABCD".into(),
            replicate: true,
            max: 100,
        };
        assert_eq!(capped.bytes(255), [100, 100]);

        let single = DeviceProfile {
            key: "ABCD".into(),
            replicate: false,
            max: 255,
        };
        assert_eq!(single.bytes(255), [255, 0]); // second byte always 0
        assert_eq!(single.bytes(128), [128, 0]); // scaled, second byte still 0

        // replicate=false combined with a capped max: both scaling and layout apply.
        let single_capped = DeviceProfile {
            key: "ABCD".into(),
            replicate: false,
            max: 200,
        };
        assert_eq!(single_capped.bytes(255), [200, 0]);
    }

    #[test]
    fn validates_api_response_for_the_requested_model() {
        let valid = ProfileResponse {
            model_id: "Mac16,9".into(),
            key: "ABCD".into(),
            format: "vv".into(),
            max: 255,
        };
        assert!(DeviceProfile::from_response(&valid, "Mac16,9").is_some());

        let mismatched = ProfileResponse {
            model_id: "Mac13,1".into(),
            key: "ABCD".into(),
            format: "vv".into(),
            max: 255,
        };
        assert!(DeviceProfile::from_response(&mismatched, "Mac16,9").is_none());
    }
}
