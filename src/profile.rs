//! Device profile — *how* to address the front LED on this Mac model.
//!
//! The SMC mechanism (write a PWM value to a 4-char key) lives in the source; the
//! per-model specifics — which key, what byte layout, the max value — are **data**,
//! not code, loaded at runtime from a profile file. Keeping these values out of the
//! binary means a new Mac model can be supported by shipping a profile, not a new
//! build, and no model-specific constants are baked into the executable.
//!
//! Profile file (plain text, one active line `KEY FORMAT MAX`):
//!   ~/Library/Application Support/LuxMini/profile
//! e.g.   `XXXX vv 255`     (vv = write [v,v];  v0 = write [v,0])
//! See `profile.example` in the repo for the format (no real key shipped).

use std::path::PathBuf;

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

fn profile_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Application Support/LuxMini/profile"))
}

impl DeviceProfile {
    /// Load the local profile, or `None` if absent/invalid (→ LED control is
    /// simply unavailable until a profile for this model is installed/fetched).
    pub fn load() -> Option<Self> {
        let text = std::fs::read_to_string(profile_path()?).ok()?;
        let line = text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with('#'))?;
        Self::parse_line(line)
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
    use super::DeviceProfile;

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
}
