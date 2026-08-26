//! `LuxMini` menu-bar app entry point: detect the Mac model, build the `AppKit` tray UI, restore last state, run the `NSApplication` loop.

mod api;
mod auth;
mod compat;
mod effects;
mod helper;
mod i18n;
mod launch_at_login;
mod led;
mod location;
mod onboarding;
mod preferences;
mod profile;
mod schedule;
mod sparkle;
mod sun;
mod ui;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

use led::{with_state, LedState, STATE};

fn main() {
    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("LuxMini must start on the main thread");
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let model = compat::get_mac_model();
    eprintln!("detected Mac model: {model}");
    let initial_state = if compat::is_pending(&model) {
        if preferences::profile_validated_for(&model)
            && profile::DeviceProfile::load_cached().is_some()
        {
            LedState::new()
        } else {
            let Some(state) = onboarding::run(mtm, &model) else {
                return;
            };
            state
        }
    } else {
        if !compat::is_supported(&model) && !compat::show_unsupported_alert(mtm, &model) {
            return;
        }
        LedState::new()
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

    // Restored last-state is applied first; the scheduler (if a rule is active)
    // then immediately re-asserts the correct value for the current time.
    if preferences::load_autodim().enabled {
        schedule::start();
    }

    sparkle::init();

    // Optional local control API (opt-in via `api.enabled`, localhost only).
    api::maybe_start();

    app.run();
}
