//! Consent-gated, pseudonymous installation and update statistics.
//!
//! A single worker orders activity and erasure requests. The installation ID
//! is random, app-specific, created only after fresh consent, and never used
//! for profiles, hardware access, advertising, or Prometheus labels.

use std::sync::{mpsc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use objc2::MainThreadMarker;
use objc2_app_kit::{NSAlert, NSApplication, NSPasteboard};
use objc2_foundation::{NSString, NSUUID};
use serde::{Deserialize, Serialize};

use crate::{compat, i18n, preferences};

#[cfg(debug_assertions)]
#[path = "diagnostics_test.rs"]
pub(crate) mod test_support;

pub const CONSENT_VERSION: i64 = 1;
const DAY_SECONDS: i64 = 86_400;
const BASE_URL: &str = "https://api.luxmini.bastiencantet.com/api/v1";
static STATE_LOCK: Mutex<()> = Mutex::new(());
static WORKER: OnceLock<Option<mpsc::SyncSender<Command>>> = OnceLock::new();
static TIMER: OnceLock<()> = OnceLock::new();

#[derive(Clone, Serialize, Deserialize)]
struct State {
    installation_id: String,
    model_id: String,
    origin: Origin,
    last_version: Option<String>,
    last_day: Option<i64>,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Origin {
    New,
    Existing,
}

#[derive(Clone, Serialize, Deserialize)]
struct Erasure {
    installation_id: String,
    model_id: String,
}

#[derive(Serialize)]
struct Request<'a> {
    event: &'a str,
    outcome: &'a str,
    model_id: &'a str,
    macos_major: u8,
    installation_id: &'a str,
    consent_version: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    app_version: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_version: Option<&'a str>,
}

enum Command {
    Activity,
    Erase(Erasure),
    #[cfg(debug_assertions)]
    Barrier(mpsc::Sender<()>),
}

pub const fn has_consent(enabled: bool, version: i64) -> bool {
    enabled && version == CONSENT_VERSION
}

/// The previous aggregate-only switch is not consent for linked statistics.
pub fn offer_consent(mtm: MainThreadMarker, existing_install: bool) {
    if should_offer_consent(
        cfg!(feature = "field-test"),
        preferences::diagnostics_consent_decided(),
    ) {
        ask_to_enable(mtm, existing_install);
    }
}

fn should_offer_consent(field_test: bool, decided: bool) -> bool {
    !field_test && !decided
}

pub fn ask_to_enable(mtm: MainThreadMarker, existing_install: bool) {
    if cfg!(feature = "field-test") {
        return;
    }
    apply_choice(show_consent(mtm), existing_install);
}

fn apply_choice(accepted: bool, existing_install: bool) {
    if cfg!(feature = "field-test") {
        return;
    }
    if !accepted {
        disable();
        return;
    }
    let Ok(guard) = STATE_LOCK.lock() else {
        return;
    };
    let previous = load_state();
    if previous.is_none() {
        preferences::reset_diagnostics_milestones();
    }
    let state = previous.unwrap_or_else(|| State {
        installation_id: NSUUID::new().UUIDString().to_string().to_ascii_lowercase(),
        model_id: metrics_model(),
        origin: if existing_install {
            Origin::Existing
        } else {
            Origin::New
        },
        last_version: None,
        last_day: None,
    });
    save_state(&state);
    preferences::save_diagnostics_consent(true);
    drop(guard);
}

fn show_consent(mtm: MainThreadMarker) -> bool {
    // Complete AppKit launch before presenting a modal choice at startup.
    // Settings calls this after the main run loop is already running.
    let app = NSApplication::sharedApplication(mtm);
    if !app.isRunning() {
        app.finishLaunching();
    }
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    let alert = consent_alert(mtm);
    #[cfg(debug_assertions)]
    if let Err(error) = test_support::capture_alert(&alert) {
        eprintln!("consent capture failed: {error}");
    }
    accepted_response(alert.runModal())
}

fn accepted_response(response: isize) -> bool {
    response == 1001
}

fn consent_alert(mtm: MainThreadMarker) -> objc2::rc::Retained<NSAlert> {
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(i18n::s(
        "Help improve LuxMini?",
        "Aider à améliorer LuxMini ?",
    )));
    alert.setInformativeText(&NSString::from_str(i18n::s(
        "Optional statistics help Bastien Cantet measure installations, updates and reliability.\n\nWith your permission, LuxMini sends a random installation ID, app version, previous version when updating, Mac model, macOS major version, daily activity and technical results to its own profile service. This links statistics over time: they are pseudonymous, not fully anonymous. No serial number, account, location, LED values or schedules are sent.\n\nLinked activity is kept for up to 90 days; inactive installation records are also removed within 90 days. Disable this option in Settings → General to stop collection, delete linked data and reset the ID (deletion is retried if offline). LuxMini works normally if you refuse. Details: luxmini.bastiencantet.com/docs#privacy",
        "Ces statistiques facultatives aident Bastien Cantet à mesurer les installations, les mises à jour et la fiabilité.\n\nAvec votre accord, LuxMini envoie un identifiant aléatoire d'installation, sa version, la version précédente lors d'une mise à jour, le modèle du Mac, la version majeure de macOS, l'activité quotidienne et des résultats techniques à son propre service de profils. Ces statistiques sont reliées dans le temps : elles sont pseudonymes, pas entièrement anonymes. Aucun numéro de série, compte, position, niveau de LED ou horaire n'est envoyé.\n\nL'activité liée est conservée au maximum 90 jours ; les installations inactives sont aussi supprimées sous 90 jours. Désactivez l'option dans Réglages → Général pour arrêter la collecte, supprimer les données liées et réinitialiser l'identifiant (suppression réessayée hors connexion). LuxMini fonctionne normalement si vous refusez. Détails : luxmini.bastiencantet.com/docs#privacy",
    )));
    // Refusal has the same interaction cost and is the default keyboard action.
    alert.addButtonWithTitle(&NSString::from_str(i18n::s("No thanks", "Non merci")));
    alert.addButtonWithTitle(&NSString::from_str(i18n::s(
        "Allow statistics",
        "Autoriser les statistiques",
    )));
    alert
}

#[cfg(debug_assertions)]
pub fn preview_consent(mtm: MainThreadMarker) {
    println!("consent_preview_accepted={}", show_consent(mtm));
}

/// Copy only after the user presses the explicitly labeled Settings button.
/// No identifier is copied automatically or included in any application log.
pub fn copy_installation_id() -> bool {
    if !preferences::diagnostics_enabled() {
        return false;
    }
    let Some(state) = load_state() else {
        return false;
    };
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    pasteboard.setString_forType(
        &NSString::from_str(&state.installation_id),
        &NSString::from_str("public.utf8-plain-text"),
    )
}

pub fn disable() {
    let Ok(guard) = STATE_LOCK.lock() else {
        return;
    };
    preferences::save_diagnostics_consent(false);
    let mut pending = pending_erasures();
    if let Some(state) = load_state() {
        if !pending
            .iter()
            .any(|item| item.installation_id == state.installation_id)
        {
            pending.push(Erasure {
                installation_id: state.installation_id,
                model_id: state.model_id,
            });
        }
    }
    save_erasures(&pending);
    preferences::save_product_metrics_state(None);
    drop(guard);
    for erasure in pending {
        enqueue(Command::Erase(erasure));
    }
}

/// Activity means an installation has `LuxMini` running, not a distinct person.
/// Hourly checks only enqueue a report when the day or app version changed.
pub fn start() {
    if cfg!(feature = "field-test") {
        return;
    }
    for erasure in pending_erasures() {
        enqueue(Command::Erase(erasure));
    }
    enqueue(Command::Activity);
    TIMER.get_or_init(|| {
        let _ = std::thread::Builder::new()
            .name("luxmini-usage-clock".into())
            .spawn(|| loop {
                std::thread::sleep(Duration::from_secs(3600));
                for erasure in pending_erasures() {
                    enqueue(Command::Erase(erasure));
                }
                enqueue(Command::Activity);
            });
    });
}

fn enqueue(command: Command) {
    if let Some(sender) = WORKER.get_or_init(|| {
        let (sender, receiver) = mpsc::sync_channel(32);
        std::thread::Builder::new()
            .name("luxmini-usage".into())
            .spawn(move || {
                for command in receiver {
                    match command {
                        Command::Activity => activity(),
                        Command::Erase(erasure) => erase(&erasure),
                        #[cfg(debug_assertions)]
                        Command::Barrier(sender) => {
                            let _ = sender.send(());
                        }
                    }
                }
            })
            .ok()
            .map(|_| sender)
    }) {
        let _ = sender.try_send(command);
    }
}

fn activity_kind(state: &State, version: &str, day: i64) -> Option<&'static str> {
    match state.last_version.as_deref() {
        None => Some(match state.origin {
            Origin::New => "new_install",
            Origin::Existing => "existing_install",
        }),
        Some(previous) if previous != version => Some("update"),
        Some(_) if state.last_day.is_none_or(|previous| day > previous) => Some("active"),
        Some(_) => None,
    }
}

fn activity() {
    let state = {
        let Ok(_guard) = STATE_LOCK.lock() else {
            return;
        };
        if !preferences::diagnostics_enabled() {
            return;
        }
        load_state()
    };
    let Some(mut state) = state else { return };
    let version = current_version();
    let day = utc_day();
    let Some(kind) = activity_kind(&state, &version, day) else {
        return;
    };
    let previous = if kind == "update" {
        state.last_version.as_deref()
    } else {
        None
    };
    let request = Request {
        event: "app_activity",
        outcome: kind,
        model_id: &state.model_id,
        macos_major: crate::telemetry::macos_major(),
        installation_id: &state.installation_id,
        consent_version: CONSENT_VERSION,
        app_version: Some(&version),
        previous_version: previous,
    };
    if !send(&request, true) {
        return;
    }
    let Ok(_guard) = STATE_LOCK.lock() else {
        return;
    };
    // A queued erasure follows any already in-flight request. Never recreate
    // local state after withdrawal or after a new identity was generated.
    if !preferences::diagnostics_enabled()
        || load_state().is_none_or(|current| current.installation_id != state.installation_id)
    {
        return;
    }
    state.last_version = Some(version);
    state.last_day = Some(state.last_day.map_or(day, |previous| previous.max(day)));
    save_state(&state);
}

fn erase(erasure: &Erasure) {
    let request = Request {
        event: "forget_installation",
        outcome: "requested",
        model_id: &erasure.model_id,
        macos_major: 0,
        installation_id: &erasure.installation_id,
        consent_version: CONSENT_VERSION,
        app_version: None,
        previous_version: None,
    };
    // Erasure is requested by the disabling action, not usage collection.
    if !send(&request, false) {
        return;
    }
    let Ok(_guard) = STATE_LOCK.lock() else {
        return;
    };
    let mut pending = pending_erasures();
    pending.retain(|item| item.installation_id != erasure.installation_id);
    save_erasures(&pending);
}

fn identity_is_consented(requested_id: &str) -> bool {
    let Ok(_guard) = STATE_LOCK.lock() else {
        return false;
    };
    matches_consent(
        preferences::diagnostics_enabled(),
        load_state()
            .as_ref()
            .map(|state| state.installation_id.as_str()),
        requested_id,
    )
}

fn matches_consent(enabled: bool, current_id: Option<&str>, requested_id: &str) -> bool {
    enabled && current_id == Some(requested_id)
}

fn send(request: &Request<'_>, require_consent: bool) -> bool {
    if require_consent && !identity_is_consented(request.installation_id) {
        return false;
    }
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .build();
    let token = agent
        .post(&format!("{}/profile-token", base_url()))
        .send_json(serde_json::json!({ "model_id": request.model_id }))
        .ok()
        .and_then(|response| response.into_json::<serde_json::Value>().ok())
        .and_then(|value| value.get("access_token")?.as_str().map(str::to_owned));
    let Some(token) = token else { return false };
    if require_consent && !identity_is_consented(request.installation_id) {
        return false;
    }
    agent
        .post(&format!("{}/telemetry", base_url()))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(request)
        .is_ok()
}

fn base_url() -> String {
    #[cfg(debug_assertions)]
    if let Some(url) = test_support::base_url() {
        return url;
    }
    BASE_URL.into()
}

fn current_version() -> String {
    #[cfg(debug_assertions)]
    if let Some(version) = test_support::version() {
        return version;
    }
    env!("CARGO_PKG_VERSION").into()
}

fn metrics_model() -> String {
    #[cfg(debug_assertions)]
    if test_support::active() {
        return "Mac999,1".into();
    }
    compat::get_mac_model()
}

fn load_state() -> Option<State> {
    serde_json::from_str(&preferences::product_metrics_state()?).ok()
}
fn save_state(state: &State) {
    if let Ok(value) = serde_json::to_string(state) {
        preferences::save_product_metrics_state(Some(&value));
    }
}
fn pending_erasures() -> Vec<Erasure> {
    preferences::pending_metrics_deletions()
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default()
}
fn save_erasures(pending: &[Erasure]) {
    if let Ok(value) = serde_json::to_string(pending) {
        preferences::save_pending_metrics_deletions(Some(&value));
    }
}
fn utc_day() -> i64 {
    #[cfg(debug_assertions)]
    if let Some(day) = test_support::day() {
        return day;
    }
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|value| i64::try_from(value.as_secs()).ok())
        .unwrap_or(0)
        / DAY_SECONDS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(origin: Origin, previous: Option<&str>, day: Option<i64>) -> State {
        State {
            installation_id: "synthetic".into(),
            model_id: "Mac999,1".into(),
            origin,
            last_version: previous.map(str::to_owned),
            last_day: day,
        }
    }
    #[test]
    fn only_explicit_allow_response_accepts() {
        for response in [-1001, -1000, -1, 0, 1, 1000, 1002, isize::MAX] {
            assert!(!accepted_response(response));
        }
        assert!(accepted_response(1001));
    }
    #[test]
    fn consent_offer_policy_is_exhaustive() {
        for field in [false, true] {
            for decided in [false, true] {
                assert_eq!(should_offer_consent(field, decided), !field && !decided);
            }
        }
    }
    #[test]
    fn clock_rollback_never_repeats_daily_activity() {
        let state = state(Origin::New, Some("0.4.3"), Some(20_000));
        for day in [-1, 0, 19_999, 20_000] {
            assert_eq!(activity_kind(&state, "0.4.3", day), None);
        }
    }
    #[test]
    fn erasure_payload_excludes_versions_and_private_fields() -> Result<(), serde_json::Error> {
        let request = Request {
            event: "forget_installation",
            outcome: "requested",
            model_id: "Mac999,1",
            macos_major: 0,
            installation_id: "synthetic",
            consent_version: 1,
            app_version: None,
            previous_version: None,
        };
        let value = serde_json::to_value(request)?;
        assert_eq!(value.as_object().map(serde_json::Map::len), Some(6));
        for field in [
            "app_version",
            "previous_version",
            "serial_number",
            "smc_key",
            "email",
            "schedule",
        ] {
            assert!(value.get(field).is_none());
        }
        Ok(())
    }
    #[test]
    fn old_aggregate_consent_does_not_enable_linked_metrics() {
        assert!(!has_consent(true, 0));
        assert!(!has_consent(false, CONSENT_VERSION));
        assert!(has_consent(true, CONSENT_VERSION));
        assert!(!has_consent(true, CONSENT_VERSION + 1));
    }
    #[test]
    fn withdrawal_and_reacceptance_do_not_authorize_the_old_identity() {
        assert!(matches_consent(true, Some("old"), "old"));
        assert!(!matches_consent(false, Some("old"), "old"));
        assert!(!matches_consent(true, Some("new"), "old"));
        assert!(!matches_consent(true, None, "old"));
    }

    #[test]
    fn an_existing_install_is_not_counted_as_a_new_user() {
        assert_eq!(
            activity_kind(&state(Origin::Existing, None, None), "0.4.3", 10),
            Some("existing_install")
        );
        assert_eq!(
            activity_kind(&state(Origin::New, None, None), "0.4.3", 10),
            Some("new_install")
        );
    }
    #[test]
    fn daily_activity_is_coalesced_but_version_changes_are_not_lost() {
        let state = state(Origin::Existing, Some("0.4.2"), Some(10));
        assert_eq!(activity_kind(&state, "0.4.2", 10), None);
        assert_eq!(activity_kind(&state, "0.4.2", 11), Some("active"));
        assert_eq!(activity_kind(&state, "0.4.3", 10), Some("update"));
        assert_eq!(activity_kind(&state, "0.4.1", 10), Some("update"));
    }
}
