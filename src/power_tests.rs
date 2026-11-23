// Controller regression tests for overlapping display and system idle events.

use super::*;
use crate::power::{Change, Event, Trigger};

fn settings(trigger: Trigger) -> preferences::SleepSettings {
    preferences::SleepSettings {
        enabled: true,
        brightness_pct: 0,
        trigger,
    }
}

#[test]
fn display_mode_restores_only_after_both_wake_events_in_either_order() {
    for kind in [LightKind::FrontLed, LightKind::RearLogo] {
        for wake in [
            [Event::SystemWake, Event::DisplaysWake],
            [Event::DisplaysWake, Event::SystemWake],
        ] {
            let mut state =
                LightState::with_output(Box::new(probe(kind)), Persistence::SessionOnly);
            state.set_brightness(153);
            let settings = settings(Trigger::DisplaySleep);
            assert_eq!(
                state.handle_power_event(Event::DisplaysSleep, settings, None),
                Some(Change::Applied(true))
            );
            assert_eq!(state.brightness, 0);
            assert_eq!(
                state.handle_power_event(Event::SystemSleep, settings, None),
                None
            );
            let _ = state.handle_power_event(wake[0], settings, None);
            assert!(state.is_sleeping());
            assert_eq!(state.brightness, 0);
            assert_eq!(state.capture_preset().brightness, 153);
            assert_eq!(
                state.handle_power_event(wake[1], settings, None),
                Some(Change::Restored(true))
            );
            assert!(!state.is_sleeping());
            assert_eq!(state.brightness, 153);
            assert_eq!(
                state.handle_power_event(Event::SystemWake, settings, None),
                None
            );
            assert_eq!(
                state.handle_power_event(Event::DisplaysWake, settings, None),
                None
            );
        }
    }
}

#[test]
fn system_mode_ignores_display_events_and_restores_on_system_wake() {
    for kind in [LightKind::FrontLed, LightKind::RearLogo] {
        let mut state = LightState::with_output(Box::new(probe(kind)), Persistence::SessionOnly);
        state.set_brightness(153);
        let settings = settings(Trigger::SystemSleep);
        assert_eq!(
            state.handle_power_event(Event::DisplaysSleep, settings, None),
            None
        );
        assert_eq!(state.brightness, 153);
        assert_eq!(
            state.handle_power_event(Event::SystemSleep, settings, None),
            Some(Change::Applied(true))
        );
        assert_eq!(state.brightness, 0);
        assert_eq!(
            state.handle_power_event(Event::SystemWake, settings, Some(51)),
            Some(Change::Restored(true))
        );
        assert_eq!(state.brightness, 51);
        assert!(!state.is_sleeping());
        assert_eq!(
            state.handle_power_event(Event::DisplaysWake, settings, None),
            None
        );
        assert_eq!(state.brightness, 51);
    }
}

#[test]
fn wake_reasserts_display_brightness_without_releasing_the_override() -> io::Result<()> {
    let output = probe(LightKind::FrontLed);
    let writes = output.writes.clone();
    let mut state = LightState::with_output(Box::new(output), Persistence::SessionOnly);
    state.set_brightness(153);
    let settings = settings(Trigger::DisplaySleep);
    let _ = state.handle_power_event(Event::SystemSleep, settings, None);
    assert_eq!(
        state.handle_power_event(Event::SystemWake, settings, None),
        Some(Change::Applied(true))
    );
    assert!(state.is_sleeping());
    assert_eq!(
        *writes.lock().map_err(|_| io::Error::other("probe lock"))?,
        vec![153, 0, 0]
    );
    let _ = state.handle_power_event(Event::DisplaysWake, settings, None);
    assert_eq!(state.brightness, 153);
    Ok(())
}

#[test]
fn awake_settings_edits_do_not_preview_brightness_on_hardware() -> io::Result<()> {
    let output = probe(LightKind::FrontLed);
    let writes = output.writes.clone();
    let mut state = LightState::with_output(Box::new(output), Persistence::SessionOnly);
    assert_eq!(
        state.update_sleep_settings(settings(Trigger::DisplaySleep), None),
        None
    );
    assert_eq!(
        state.update_sleep_settings(settings(Trigger::SystemSleep), None),
        None
    );
    assert!(writes
        .lock()
        .map_err(|_| io::Error::other("probe lock"))?
        .is_empty());
    Ok(())
}

#[test]
fn disabling_or_changing_the_trigger_releases_an_active_display_override() {
    for disable in [false, true] {
        let mut state = LightState::with_output(
            Box::new(probe(LightKind::FrontLed)),
            Persistence::SessionOnly,
        );
        state.set_brightness(153);
        let _ =
            state.handle_power_event(Event::DisplaysSleep, settings(Trigger::DisplaySleep), None);
        let mut updated = settings(if disable {
            Trigger::DisplaySleep
        } else {
            Trigger::SystemSleep
        });
        updated.enabled = !disable;
        assert_eq!(
            state.update_sleep_settings(updated, None),
            Some(Change::Restored(true))
        );
        assert!(!state.is_sleeping());
        assert_eq!(state.brightness, 153);
    }
}

#[test]
fn updating_the_idle_level_preserves_the_original_manual_setting() {
    let mut state = LightState::with_output(
        Box::new(probe(LightKind::FrontLed)),
        Persistence::SessionOnly,
    );
    state.set_brightness(153);
    let mut settings = settings(Trigger::DisplaySleep);
    let _ = state.handle_power_event(Event::DisplaysSleep, settings, None);
    settings.brightness_pct = 5;
    assert_eq!(
        state.update_sleep_settings(settings, None),
        Some(Change::Applied(true))
    );
    assert_eq!(state.brightness, 13);
    assert_eq!(state.capture_preset().brightness, 153);
    let _ = state.handle_power_event(Event::DisplaysWake, settings, None);
    assert_eq!(state.brightness, 153);
}

#[test]
fn display_write_failure_is_reported_without_inventing_a_success() {
    let output = probe(LightKind::FrontLed);
    let reject = output.reject.clone();
    let mut state = LightState::with_output(Box::new(output), Persistence::SessionOnly);
    state.set_brightness(153);
    reject.store(true, Ordering::Relaxed);
    let settings = settings(Trigger::DisplaySleep);
    assert_eq!(
        state.handle_power_event(Event::DisplaysSleep, settings, None),
        Some(Change::Applied(false))
    );
    assert_eq!(state.brightness, 153);
    assert!(state.is_sleeping());
    assert!(!state.is_ready());
    reject.store(false, Ordering::Relaxed);
    let _ = state.handle_power_event(Event::DisplaysWake, settings, None);
    assert_eq!(state.brightness, 153);
    assert!(state.is_ready());
}
