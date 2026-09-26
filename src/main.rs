//! `LuxMini` menu-bar app entry point: detect the Mac model, build the `AppKit` tray UI, restore last state, run the `NSApplication` loop.

mod api;
mod auth;
mod compat;
mod effects;
mod helper;
mod helper_protocol;
mod i18n;
mod launch_at_login;
mod led;
mod location;
mod onboarding;
mod preferences;
mod profile;
mod schedule;
#[cfg(all(feature = "direct", not(feature = "field-test")))]
mod sparkle;
mod sun;
mod telemetry;
mod ui;
mod welcome;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

use led::{with_state, LedState, STATE};

#[cfg(not(feature = "direct"))]
compile_error!("enable the direct distribution feature");

#[cfg(all(feature = "field-test", not(feature = "direct")))]
compile_error!("the field-test feature requires the direct distribution feature");

const fn should_run_setup(eligible: bool, verified: bool, must_recheck: bool) -> bool {
    eligible && (!verified || must_recheck)
}

#[allow(clippy::too_many_lines)] // Debug preview modes are kept close to app startup.
fn main() {
    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("LuxMini must start on the main thread");
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    #[cfg(all(debug_assertions, feature = "direct", not(feature = "field-test")))]
    if std::env::var_os("LUXMINI_CHECK_SPARKLE_LOAD").is_some() {
        println!("sparkle_framework_loads={}", sparkle::framework_loads());
        return;
    }

    #[cfg(debug_assertions)]
    if std::env::var_os("LUXMINI_PREVIEW_WELCOME").is_some() {
        welcome::preview(mtm);
        return;
    }

    #[cfg(debug_assertions)]
    if std::env::var_os("LUXMINI_CHECK_SAVE_HIT_TEST").is_some() {
        let handler = ui::handler::Handler::new(mtm);
        ui::settings::open(&handler, mtm);
        println!(
            "apply_automation_hit={}",
            ui::settings::save_receives_pointer_hits(&handler, 1)
        );
        return;
    }

    #[cfg(debug_assertions)]
    if let Some(directory) = std::env::var_os("LUXMINI_PREVIEW_SETTINGS_EXPORT") {
        let handler = ui::handler::Handler::new(mtm);
        ui::settings::open(&handler, mtm);
        let directory = std::path::PathBuf::from(directory);
        if let Err(error) = ui::settings::export_previews(&handler, &directory) {
            eprintln!("settings preview export failed: {error}");
        }
        return;
    }

    #[cfg(debug_assertions)]
    if std::env::var_os("LUXMINI_PREVIEW_SETTINGS_SHOW").is_some() {
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        let handler = ui::handler::Handler::new(mtm);
        ui::settings::open(&handler, mtm);
        app.run();
        return;
    }

    let model = compat::get_mac_model();
    eprintln!("detected Mac model: {model}");
    telemetry::note_launch(&model);
    let pending = compat::is_pending(&model);
    let supported = compat::is_supported(&model);
    if !pending && !supported && !compat::show_unsupported_alert(mtm, &model) {
        return;
    }
    let needs_validation = pending
        && !(preferences::profile_validated_for(&model)
            && profile::DeviceProfile::load_cached().is_some());
    let initial_state = if should_run_setup(
        supported || pending,
        preferences::setup_verified_for(&model),
        preferences::setup_pending_for(&model) || needs_validation,
    ) {
        let Some(state) = welcome::run(mtm, &model, pending) else {
            return;
        };
        state
    } else {
        let state = LedState::new();
        if (supported || pending) && !state.helper_ready() {
            eprintln!("LED control unavailable; reopening setup");
            let Some(recovered) = welcome::run(mtm, &model, pending) else {
                return;
            };
            recovered
        } else {
            state
        }
    };

    match STATE.lock() {
        Ok(mut state) => *state = Some(initial_state),
        Err(err) => {
            eprintln!("LED state lock poisoned at startup: {err}");
            return;
        }
    }
    let app_handle = ui::build_app(mtm);

    // Handler doubles as the NSApplication delegate so reopening LuxMini brings the icon back.
    // SAFETY: setDelegate: takes an NSApplicationDelegate; handler outlives the run loop.
    unsafe {
        let _: () = objc2::msg_send![&app, setDelegate: &*app_handle.handler];
    }
    if preferences::load_hide_icon() {
        app_handle.handler.set_icon_hidden(true);
    }

    if let Some(last) = preferences::load_last_state() {
        with_state(|s| s.apply_preset(&last));
        app_handle.handler.refresh_ui();
    }
    app_handle.handler.show_control_error_if_any();

    // Restored last-state is applied first; the scheduler (if a rule is active)
    // then immediately re-asserts the correct value for the current time.
    if preferences::load_autodim().enabled {
        schedule::start();
    }

    let _helper_feedback_timer = app_handle.handler.start_helper_feedback_timer();
    led::start_helper_monitor();

    #[cfg(all(feature = "direct", not(feature = "field-test")))]
    sparkle::init();
    // Optional local control API (opt-in via `api.enabled`, localhost only).
    api::maybe_start();

    app.run();
}

#[cfg(test)]
mod setup_tests {
    use super::should_run_setup;

    #[test]
    fn existing_install_without_setup_verification_runs_setup() {
        assert!(should_run_setup(true, false, false));
    }

    #[test]
    fn verified_install_starts_normally_unless_setup_was_interrupted() {
        assert!(!should_run_setup(true, true, false));
        assert!(should_run_setup(true, true, true));
    }

    #[test]
    fn pending_model_requires_visual_validation() {
        assert!(should_run_setup(true, true, true));
        assert!(!should_run_setup(false, false, true));
    }
}
