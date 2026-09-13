//! Mac model detection and the 'unsupported hardware' warning dialog.

use objc2::MainThreadMarker;
use objc2_app_kit::{NSAlert, NSApplication, NSImage};
use objc2_foundation::NSString;
use std::process::Command;

const SUPPORTED_EXACT: &[&str] = &[
    // Mac mini (Apple Silicon)
    "Mac14,3",  // M2
    "Mac14,12", // M2 Pro
    // Mac Studio — same front-LED mechanism across all models (community-tested).
    "Mac13,1",  // M1 Max (2022)
    "Mac13,2",  // M1 Ultra (2022)
    "Mac14,13", // M2 Max (2023)
    "Mac14,14", // M2 Ultra (2023)
    "Mac15,14", // M3 Ultra (2025)
    "Mac16,9",  // M4 Max (2025)
];

// Recognized ahead of availability, but deliberately not enabled until a real
// machine confirms the candidate LED profile. Keep this separate from
// `SUPPORTED_EXACT` so a release cannot silently claim hardware validation.
const PENDING_EXACT: &[&str] = &[
    // Re-validation requested after two independent macOS users reported that
    // the approved profile produced no visible LED response.
    "Mac16,10", // Mac mini M4 (2024)
    "Mac16,11", // Mac mini M4 Pro (2024)
    "Mac18,5",  // Mac mini M6 (2026)
    "Mac17,16", // Mac mini M5 Pro (2026)
    "Mac17,14", // Mac Studio M5 Max (2026)
    "Mac17,15", // Mac Studio M5 Ultra (2026)
];

/// `runModal` returns this when the second-added button ("Open Anyway") is clicked.
const NS_ALERT_SECOND_BUTTON_RETURN: isize = 1001;

/// The Mac's hardware model identifier (e.g. `Macmini9,1`), or an empty string
/// if `sysctl hw.model` fails — which `is_supported` then treats as unsupported.
#[must_use]
pub fn get_mac_model() -> String {
    Command::new("sysctl")
        .args(["-n", "hw.model"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

#[must_use]
pub fn is_supported(model: &str) -> bool {
    model.starts_with("Macmini") || SUPPORTED_EXACT.contains(&model)
}

#[must_use]
pub fn is_pending(model: &str) -> bool {
    PENDING_EXACT.contains(&model)
}

pub fn show_unsupported_alert(mtm: MainThreadMarker, model: &str) -> bool {
    let app = NSApplication::sharedApplication(mtm);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);

    let alert = NSAlert::new(mtm);
    if let Some(icon) = NSImage::imageNamed(&NSString::from_str("NSCaution")) {
        // SAFETY: setIcon: is a standard AppKit setter on the main thread.
        unsafe { alert.setIcon(Some(&icon)) };
    }

    let model_display = if model.is_empty() {
        crate::i18n::s("unknown", "inconnu")
    } else {
        model
    };
    alert.setMessageText(&NSString::from_str(crate::i18n::s(
        "Unsupported Mac",
        "Mac non supporté",
    )));
    let info = if is_pending(model) && crate::i18n::fr() {
        format!(
            "Ce nouveau Mac ({model_display}) est reconnu, mais son profil LED attend une validation matérielle."
        )
    } else if is_pending(model) {
        format!(
            "This new Mac ({model_display}) is recognized, but its LED profile is awaiting hardware validation."
        )
    } else if crate::i18n::fr() {
        format!("Ce modèle ({model_display}) n'est pas dans la liste des Mac supportés.")
    } else {
        format!("This model ({model_display}) is not in the list of supported Macs.")
    };
    alert.setInformativeText(&NSString::from_str(&info));
    alert.addButtonWithTitle(&NSString::from_str(crate::i18n::s("Quit", "Quitter")));
    alert.addButtonWithTitle(&NSString::from_str(crate::i18n::s(
        "Open Anyway",
        "Ouvrir quand même",
    )));

    alert.runModal() == NS_ALERT_SECOND_BUTTON_RETURN
}

#[cfg(test)]
mod tests {
    use super::{is_pending, is_supported, SUPPORTED_EXACT};

    #[test]
    fn mac_mini_prefix_is_supported() {
        assert!(is_supported("Macmini9,1"));
        assert!(is_supported("Macmini"));
    }

    #[test]
    fn exact_models_are_supported() {
        for m in SUPPORTED_EXACT {
            assert!(is_supported(m), "{m} should be supported");
        }
    }

    #[test]
    fn mac_studio_is_supported() {
        for m in [
            "Mac13,1", "Mac13,2", "Mac14,13", "Mac14,14", "Mac15,14", "Mac16,9",
        ] {
            assert!(is_supported(m), "{m} (Mac Studio) should be supported");
        }
    }

    #[test]
    fn new_2026_macs_are_recognized_as_pending() {
        for m in ["Mac18,5", "Mac17,16", "Mac17,14", "Mac17,15"] {
            assert!(
                !is_supported(m),
                "{m} must not be enabled before validation"
            );
            assert!(super::is_pending(m), "{m} should be recognized as pending");
        }
    }

    #[test]
    fn m4_minis_require_visual_profile_revalidation() {
        for model in ["Mac16,10", "Mac16,11"] {
            assert!(is_pending(model));
            assert!(!is_supported(model));
        }
    }

    #[test]
    fn unrelated_models_are_unsupported() {
        assert!(!is_supported("MacBookPro18,1"));
        assert!(!is_supported(""));
    }
}
