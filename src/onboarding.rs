//! Interactive LED-profile discovery for pending or manually retested Macs.

use std::thread;
use std::time::Duration;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSAlert, NSApplication};
use objc2_foundation::NSString;

use crate::helper::Helper;
use crate::led::LedState;
use crate::profile::{CandidateProfile, DeviceProfile};

const FIRST_BUTTON: isize = 1000;
const MAX_CANDIDATES: u8 = 3;

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

/// Run a numbered pulse pattern and restore the exact bytes read before it.
/// Discovery accepts only two-byte keys, matching every allowlisted LED profile.
fn run_pattern(helper: &mut Helper, slot: u8) -> std::io::Result<()> {
    let original = helper.read_profile_raw()?;
    if original.len() != 2 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "candidate SMC key is not two bytes",
        ));
    }

    let result = (0..slot).try_for_each(|_| {
        helper.write_led_test(255)?;
        thread::sleep(Duration::from_millis(260));
        helper.write_led_test(24)?;
        thread::sleep(Duration::from_millis(360));
        Ok(())
    });
    thread::sleep(Duration::from_millis(180));
    let restore = helper.restore_profile_raw(&original);
    result.and(restore)
}

fn choose_candidate(
    mtm: MainThreadMarker,
    matches: &[(u8, DeviceProfile)],
) -> Option<DeviceProfile> {
    if matches.len() == 1 {
        return matches.first().map(|(_, profile)| profile.clone());
    }

    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(crate::i18n::s(
        "Several profiles worked",
        "Plusieurs profils ont fonctionné",
    )));
    alert.setInformativeText(&NSString::from_str(crate::i18n::s(
        "Select the test whose pulses were the clearest. LuxMini will use only that profile on this Mac.",
        "Sélectionnez le test dont les pulsations étaient les plus nettes. LuxMini utilisera uniquement ce profil sur ce Mac.",
    )));
    for (slot, _) in matches {
        alert.addButtonWithTitle(&NSString::from_str(&format!(
            "{} {slot}",
            crate::i18n::s("Profile", "Profil")
        )));
    }
    alert.addButtonWithTitle(&NSString::from_str(crate::i18n::s("Cancel", "Annuler")));
    let index = alert.runModal() - FIRST_BUTTON;
    usize::try_from(index)
        .ok()
        .and_then(|index| matches.get(index))
        .map(|(_, profile)| profile.clone())
}

fn confirm_start(mtm: MainThreadMarker, model: &str) -> bool {
    let details = if crate::i18n::fr() {
        format!(
            "LuxMini cherche comment accéder à la LED sur ce Mac ({model}). L'application testera au maximum trois profils LED connus. Chaque test produit 1, 2 ou 3 pulsations, puis restaure exactement la valeur précédente. Aucune autre clé SMC n'est testée. Seuls le modèle, le numéro du candidat et votre réponse sont envoyés."
        )
    } else {
        format!(
            "LuxMini is determining how to access the LED on this Mac ({model}). The app will test at most three known LED profiles. Each test produces 1, 2, or 3 pulses, then restores the exact previous value. No other SMC key is tested. Only the model, candidate number, and your answer are sent."
        )
    };
    alert(
        mtm,
        crate::i18n::s("Detect LED access", "Détecter l'accès à la LED"),
        &details,
        crate::i18n::s("Start Detection", "Démarrer la détection"),
        Some(crate::i18n::s("Cancel", "Annuler")),
    ) == FIRST_BUTTON
}

fn fetch_candidates(mtm: MainThreadMarker, model: &str) -> Option<Vec<CandidateProfile>> {
    let Some(first) = DeviceProfile::fetch_candidate(model, 1) else {
        alert(
            mtm,
            crate::i18n::s("No candidate available", "Aucun candidat disponible"),
            crate::i18n::s(
                "LuxMini has no allowlisted LED candidate for this Mac yet. No SMC write was attempted.",
                "LuxMini ne dispose pas encore de candidat LED autorisé pour ce Mac. Aucune écriture SMC n'a été tentée.",
            ),
            "OK",
            None,
        );
        return None;
    };

    let total = first.total.min(MAX_CANDIDATES);
    let mut candidates = vec![first];
    for slot in 2..=total {
        if let Some(candidate) = DeviceProfile::fetch_candidate(model, slot) {
            candidates.push(candidate);
        }
    }
    Some(candidates)
}

fn test_candidate(
    mtm: MainThreadMarker,
    model: &str,
    total: u8,
    candidate: CandidateProfile,
) -> Result<Option<(u8, DeviceProfile)>, ()> {
    let CandidateProfile { slot, profile, .. } = candidate;
    let mut helper = Helper::spawn_with_profile(Some(profile.clone())).map_err(|error| {
        eprintln!("candidate LED helper unavailable: {error}");
        let _ = DeviceProfile::report_validation(model, slot, "technical_error");
        alert(
            mtm,
            crate::i18n::s("LED test failed", "Échec du test LED"),
            crate::i18n::s(
                "LuxMini could not start the privileged LED helper. No profile was changed.",
                "LuxMini n'a pas pu démarrer l'assistant privilégié de la LED. Aucun profil n'a été modifié.",
            ),
            "OK",
            None,
        );
    })?;
    if let Err(error) = run_pattern(&mut helper, slot) {
        eprintln!("candidate LED pattern {slot} failed: {error}");
        let _ = DeviceProfile::report_validation(model, slot, "technical_error");
        return Ok(None);
    }

    let prompt = if crate::i18n::fr() {
        format!("Avez-vous vu la LED avant produire exactement {slot} pulsation(s) ?")
    } else {
        format!("Did you see the front LED produce exactly {slot} pulse(s)?")
    };
    let saw_pulses = alert(
        mtm,
        &format!("{} {slot}/{total}", crate::i18n::s("LED test", "Test LED")),
        &prompt,
        crate::i18n::s("Yes", "Oui"),
        Some(crate::i18n::s("No", "Non")),
    ) == FIRST_BUTTON;
    let _ = DeviceProfile::report_validation(model, slot, if saw_pulses { "yes" } else { "no" });
    Ok(saw_pulses.then_some((slot, profile)))
}

fn show_no_match(mtm: MainThreadMarker) {
    alert(
        mtm,
        crate::i18n::s("No LED profile matched", "Aucun profil LED compatible"),
        crate::i18n::s(
            "LuxMini kept the previous configuration. Your answers will help prepare another safe candidate.",
            "LuxMini a conservé la configuration précédente. Vos réponses aideront à préparer un autre candidat sûr.",
        ),
        "OK",
        None,
    );
}

fn run_discovery(mtm: MainThreadMarker, model: &str, manual: bool) -> Option<LedState> {
    if !confirm_start(mtm, model) {
        return None;
    }
    let candidates = fetch_candidates(mtm, model)?;
    let total = u8::try_from(candidates.len()).ok()?.min(MAX_CANDIDATES);
    let mut matches = Vec::new();
    for candidate in candidates {
        if let Some(candidate_match) = test_candidate(mtm, model, total, candidate).ok()? {
            matches.push(candidate_match);
        }
    }
    let Some(selected) = choose_candidate(mtm, &matches) else {
        if matches.is_empty() {
            show_no_match(mtm);
        }
        return None;
    };
    let selected_slot = matches
        .iter()
        .find_map(|(slot, profile)| (profile == &selected).then_some(*slot))?;
    if let Err(error) = selected.cache_validated(model) {
        eprintln!("cannot cache validated candidate profile: {error}");
        let _ = DeviceProfile::report_validation(model, selected_slot, "technical_error");
        return None;
    }
    crate::preferences::save_validated_profile_model(model);
    let _ = DeviceProfile::report_validation(model, selected_slot, "selected");

    let helper = match Helper::spawn_with_profile(Some(selected)) {
        Ok(helper) => helper,
        Err(error) => {
            eprintln!("selected LED helper unavailable: {error}");
            return None;
        }
    };
    if manual {
        alert(
            mtm,
            crate::i18n::s("LED access detected", "Accès à la LED détecté"),
            crate::i18n::s(
                "The selected profile is now active on this Mac.",
                "Le profil sélectionné est maintenant actif sur ce Mac.",
            ),
            "OK",
            None,
        );
    }
    Some(LedState::with_helper(helper))
}

pub fn run(mtm: MainThreadMarker, model: &str) -> Option<LedState> {
    run_discovery(mtm, model, false)
}

pub fn run_manual(mtm: MainThreadMarker, model: &str) -> Option<LedState> {
    run_discovery(mtm, model, true)
}

#[cfg(test)]
mod tests {
    use super::MAX_CANDIDATES;

    #[test]
    fn discovery_is_bounded_to_three_candidates() {
        assert_eq!(MAX_CANDIDATES, 3);
    }
}
