//! Interactive hardware validation for recognized, pending Mac models.

use std::thread;
use std::time::Duration;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSAlert, NSApplication};
use objc2_foundation::NSString;

use crate::helper::Helper;
use crate::led::LedState;
use crate::profile::DeviceProfile;

const FIRST_BUTTON: isize = 1000;

fn alert(
    mtm: MainThreadMarker,
    title: &str,
    text: &str,
    first: &str,
    second: Option<&str>,
) -> isize {
    let app = NSApplication::sharedApplication(mtm);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(&NSString::from_str(text));
    alert.addButtonWithTitle(&NSString::from_str(first));
    if let Some(label) = second {
        alert.addButtonWithTitle(&NSString::from_str(label));
    }
    alert.runModal()
}

fn run_fade(helper: &mut Helper) -> std::io::Result<()> {
    let values = [
        255, 224, 192, 160, 128, 96, 64, 32, 64, 96, 128, 160, 192, 224, 255,
    ];
    let result = values.into_iter().try_for_each(|value| {
        helper.write_led(value)?;
        thread::sleep(Duration::from_millis(90));
        Ok(())
    });
    // Always attempt to leave the LED fully lit, including after a failed write.
    let restore = helper.write_led(255);
    result.and(restore)
}

pub fn run(mtm: MainThreadMarker, model: &str) -> Option<LedState> {
    let title = crate::i18n::s("New Mac detected", "Nouveau Mac détecté");
    let details = if crate::i18n::fr() {
        format!(
            "LuxMini reconnaît ce modèle ({model}), mais son profil LED attend une validation matérielle. Le test fera varier doucement la LED puis la remettra à pleine luminosité. Après le test, LuxMini enverra uniquement le modèle et votre réponse Yes/No — aucun numéro de série ni identifiant d’installation."
        )
    } else {
        format!(
            "LuxMini recognizes this model ({model}), but its LED profile is awaiting hardware validation. The test will gently fade the LED and then restore full brightness. After the test, LuxMini sends only the model and your Yes/No answer — no serial number or installation ID."
        )
    };
    if alert(
        mtm,
        title,
        &details,
        crate::i18n::s("Start LED Test", "Démarrer le test LED"),
        Some(crate::i18n::s("Not Now", "Pas maintenant")),
    ) != FIRST_BUTTON
    {
        return None;
    }

    let Some(profile) = DeviceProfile::fetch_candidate(model) else {
        alert(
            mtm,
            crate::i18n::s("Profile unavailable", "Profil indisponible"),
            crate::i18n::s(
                "LuxMini could not download the candidate profile. Check your connection and try again.",
                "LuxMini n’a pas pu télécharger le profil candidat. Vérifiez votre connexion et réessayez.",
            ),
            "OK",
            None,
        );
        return None;
    };
    let mut helper = match Helper::spawn_with_profile(Some(profile.clone())) {
        Ok(helper) => helper,
        Err(error) => {
            eprintln!("candidate LED helper unavailable: {error}");
            let _ = DeviceProfile::report_validation(model, "technical_error");
            alert(
                mtm,
                crate::i18n::s("LED test failed", "Échec du test LED"),
                crate::i18n::s(
                    "LuxMini could not start the LED test. No profile was enabled.",
                    "LuxMini n’a pas pu démarrer le test LED. Aucun profil n’a été activé.",
                ),
                "OK",
                None,
            );
            return None;
        }
    };

    if let Err(error) = run_fade(&mut helper) {
        eprintln!("candidate LED fade failed: {error}");
        let _ = DeviceProfile::report_validation(model, "technical_error");
        alert(
            mtm,
            crate::i18n::s("LED test failed", "Échec du test LED"),
            crate::i18n::s(
                "The LED test could not complete. The LED was restored to full brightness when possible, and no profile was enabled.",
                "Le test LED n’a pas pu se terminer. La LED a été remise à pleine luminosité lorsque possible et aucun profil n’a été activé.",
            ),
            "OK",
            None,
        );
        return None;
    }

    let saw_fade = alert(
        mtm,
        crate::i18n::s("Did the LED fade?", "La LED a-t-elle varié ?"),
        crate::i18n::s(
            "Did you see the front LED smoothly fade down and back up?",
            "Avez-vous vu la LED avant baisser progressivement puis remonter ?",
        ),
        crate::i18n::s("Yes", "Oui"),
        Some(crate::i18n::s("No", "Non")),
    ) == FIRST_BUTTON;

    if !saw_fade {
        let _ = DeviceProfile::report_validation(model, "no");
        return None;
    }
    if let Err(error) = profile.cache_validated(model) {
        eprintln!("cannot cache validated candidate profile: {error}");
        let _ = DeviceProfile::report_validation(model, "technical_error");
        return None;
    }
    crate::preferences::save_validated_profile_model(model);
    let _ = DeviceProfile::report_validation(model, "yes");
    Some(LedState::with_helper(helper))
}
