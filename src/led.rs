//! LED state machine and the global STATE mutex; mediates all writes through the privileged helper.

use crate::effects::run_effect;
use crate::helper::Helper;
use crate::preferences;
use crate::profile::DeviceProfile;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Blink,
    BlinkFast,
    Pulse,
    Sos,
    Strobe,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LedAccess {
    Ready,
    WriteFailed,
}

pub struct LedState {
    helper: Option<Helper>,
    control_error: Option<String>,
    // A live helper is not proof that its current SMC profile can write the LED.
    led_access: LedAccess,
    // LIVE state — what the LED shows now (the scheduler may dim/off it via `apply_auto`).
    is_on: bool,
    brightness: u8,
    // MANUAL intent — what the user last chose; the persisted day baseline the schedule
    // restores to. `apply_auto` never touches it, so a manual OFF is respected.
    manual_on: bool,
    manual_brightness: u8,
    effect_stop: Option<Arc<AtomicBool>>,
    current_effect: Option<Effect>,
    auto_restart_used: bool,
}

/// Resolve an auto-dim decision into `(brightness, is_on)`: `Some(v)` forces `v`
/// (night rule); `None` releases to the user's manual intent — off if they turned
/// it off. Pure, so "don't fight the user" is unit-tested.
const fn resolve_auto(target: Option<u8>, manual_on: bool, manual_brightness: u8) -> (u8, bool) {
    match target {
        Some(v) => (v, v > 0),
        None if manual_on => (manual_brightness, true),
        None => (0, false),
    }
}

impl LedState {
    pub fn new() -> Self {
        let (helper, control_error) = match Helper::spawn().and_then(|mut helper| {
            let bytes = helper.read_profile_raw()?;
            if bytes.len() != 2 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "LED profile key must contain exactly two bytes",
                ));
            }
            Ok(helper)
        }) {
            Ok(h) => (Some(h), None),
            Err(e) => {
                eprintln!("helper not available: {e} (run `make setup`)");
                (None, Some(e.to_string()))
            }
        };
        Self {
            helper,
            control_error,
            led_access: LedAccess::Ready,
            is_on: true,
            brightness: 0xff,
            manual_on: true,
            manual_brightness: 0xff,
            effect_stop: None,
            current_effect: None,
            auto_restart_used: false,
        }
    }

    pub const fn with_helper(helper: Helper) -> Self {
        Self {
            helper: Some(helper),
            control_error: None,
            led_access: LedAccess::Ready,
            is_on: true,
            brightness: 0xff,
            manual_on: true,
            manual_brightness: 0xff,
            effect_stop: None,
            current_effect: None,
            auto_restart_used: false,
        }
    }

    pub const fn helper_ready(&self) -> bool {
        self.helper.is_some() && matches!(self.led_access, LedAccess::Ready)
    }

    pub fn check_helper_health(&mut self) {
        let Some(helper) = self.helper.as_mut() else {
            return;
        };
        if let Err(error) = helper.check_running() {
            eprintln!("LED helper health check failed: {error}");
            let effect_was_active = self.current_effect.is_some();
            let restore = if effect_was_active {
                self.manual_brightness_if_on()
            } else {
                self.brightness
            };
            self.recover_helper(&error, restore, effect_was_active);
        }
    }

    #[cfg(feature = "field-test")]
    pub fn test_helper_connection(&mut self) -> io::Result<()> {
        let result = self.helper.as_mut().map_or_else(
            || Err(io::Error::other("LED helper unavailable")),
            |helper| {
                helper.ping()?;
                let bytes = helper.read_profile_raw()?;
                if bytes.len() != 2 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "LED profile key must contain exactly two bytes",
                    ));
                }
                Ok(())
            },
        );
        if result.is_err() {
            self.led_access = LedAccess::WriteFailed;
        }
        result
    }

    #[cfg(feature = "field-test")]
    pub fn test_helper_restart(&mut self) -> io::Result<()> {
        if self.auto_restart_used {
            return Err(io::Error::other(
                "automatic restart already used in this app session",
            ));
        }
        self.helper
            .as_mut()
            .ok_or_else(|| io::Error::other("LED helper unavailable"))?
            .terminate()?;
        self.check_helper_health();
        if self.helper_ready() {
            Ok(())
        } else {
            Err(io::Error::other(
                self.control_error
                    .as_deref()
                    .unwrap_or("LED helper restart failed"),
            ))
        }
    }

    const fn manual_brightness_if_on(&self) -> u8 {
        if self.manual_on {
            self.manual_brightness
        } else {
            0
        }
    }

    fn transport_failed(error: &io::Error) -> bool {
        matches!(
            error.kind(),
            io::ErrorKind::BrokenPipe
                | io::ErrorKind::UnexpectedEof
                | io::ErrorKind::TimedOut
                | io::ErrorKind::InvalidData
        )
    }

    fn start_verified_helper(profile: DeviceProfile, value: u8) -> io::Result<Helper> {
        let mut helper = Helper::spawn_unattended(profile)?;
        let result = helper.read_profile_raw().and_then(|bytes| {
            if bytes.len() != 2 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "LED profile key must contain exactly two bytes",
                ));
            }
            helper.write_led(value)
        });
        if let Err(error) = result {
            let _ = helper.terminate();
            return Err(error);
        }
        Ok(helper)
    }

    /// Retry once without requesting admin rights. Further failures require a
    /// deliberate user-initiated setup check instead of a restart loop.
    fn recover_helper(&mut self, error: &io::Error, value: u8, effect_was_active: bool) -> bool {
        self.stop_effect();
        let profile = self.helper.as_ref().map(Helper::profile);
        let termination = self
            .helper
            .take()
            .map_or(Ok(()), |mut helper| helper.terminate());
        let restart = if let Err(termination_error) = termination {
            Err(termination_error)
        } else if self.auto_restart_used {
            Err(io::Error::other("automatic restart limit reached"))
        } else {
            self.auto_restart_used = true;
            profile.map_or_else(
                || Err(io::Error::other("LED helper profile unavailable")),
                |profile| Self::start_verified_helper(profile, value),
            )
        };
        match restart {
            Ok(helper) => {
                eprintln!("LED helper restarted after failure: {error}");
                self.helper = Some(helper);
                self.led_access = LedAccess::Ready;
                if effect_was_active {
                    self.is_on = self.manual_on;
                    self.brightness = value;
                    self.persist();
                    self.control_error =
                        Some("The LED helper restarted, but the active effect stopped".to_owned());
                    false
                } else {
                    self.control_error = None;
                    true
                }
            }
            Err(restart_error) => {
                eprintln!("LED helper recovery failed: {restart_error}");
                if effect_was_active {
                    self.persist();
                }
                self.control_error = Some(format!(
                    "{error}; automatic recovery failed: {restart_error}"
                ));
                false
            }
        }
    }

    fn write(&mut self, value: u8) -> bool {
        let result = self.helper.as_mut().map_or_else(
            || Err(std::io::Error::other("LED helper unavailable")),
            |helper| helper.write_led(value),
        );
        match result {
            Ok(()) => {
                self.control_error = None;
                self.led_access = LedAccess::Ready;
                true
            }
            Err(error) => {
                eprintln!("LED write failed: {error}");
                if self.helper.is_some() && Self::transport_failed(&error) {
                    return self.recover_helper(&error, value, self.current_effect.is_some());
                }
                self.led_access = LedAccess::WriteFailed;
                let effect_was_active = self.current_effect.is_some();
                self.stop_effect();
                if effect_was_active {
                    self.persist();
                }
                self.control_error = Some(error.to_string());
                false
            }
        }
    }

    pub fn take_control_error(&mut self) -> Option<String> {
        self.control_error.take()
    }

    pub const fn control_failed(&self) -> bool {
        self.control_error.is_some()
    }

    fn stop_effect(&mut self) {
        if let Some(flag) = self.effect_stop.take() {
            flag.store(true, Ordering::Relaxed);
        }
        self.current_effect = None;
    }

    pub fn start_effect(&mut self, effect: Effect) {
        self.stop_effect();
        if !self.write(0xff) {
            return;
        }
        self.current_effect = Some(effect);
        let flag = Arc::new(AtomicBool::new(false));
        self.effect_stop = Some(flag.clone());
        std::thread::spawn(move || run_effect(effect, flag));
        self.persist();
    }

    pub fn clear_effect(&mut self) {
        self.stop_effect();
        // Return the LED to the user's manual intent, not the live transient value.
        let v = if self.manual_on {
            self.manual_brightness
        } else {
            0
        };
        if !self.write(v) {
            return;
        }
        self.is_on = self.manual_on;
        self.brightness = v;
        self.persist();
    }

    fn persist(&self) {
        preferences::save_last_state(&self.capture_preset());
    }

    /// Snapshot the user's MANUAL intent (not the transient live value), so presets and
    /// last-state record what the user chose and survive a reboot.
    pub const fn capture_preset(&self) -> preferences::Preset {
        preferences::Preset {
            brightness: self.manual_brightness,
            is_on: self.manual_on,
            effect: self.current_effect,
        }
    }

    /// Stop a running effect before profile discovery while preserving the
    /// user's complete manual intent so it can be resumed afterward.
    pub fn pause_for_profile_discovery(&mut self) -> preferences::Preset {
        let preset = self.capture_preset();
        self.stop_effect();
        preset
    }

    #[allow(clippy::trivially_copy_pass_by_ref)] // mirrors preferences::save_*(&Preset) for call-site symmetry
    pub fn apply_preset(&mut self, p: &preferences::Preset) {
        self.stop_effect();
        let manual_brightness = if p.brightness > 0 {
            p.brightness
        } else {
            self.manual_brightness
        };
        let live = if p.is_on { p.brightness } else { 0 };
        if let Some(e) = p.effect {
            self.start_effect(e); // persists
            if self.control_error.is_none() {
                self.manual_on = p.is_on;
                self.manual_brightness = manual_brightness;
                self.persist();
            }
        } else {
            if !self.write(live) {
                return;
            }
            self.manual_on = p.is_on;
            self.manual_brightness = manual_brightness;
            self.is_on = p.is_on;
            self.brightness = live;
            self.persist();
        }
    }

    pub fn set_brightness(&mut self, value: u8) {
        self.stop_effect();
        if !self.write(value) {
            return;
        }
        self.manual_on = value > 0;
        if value > 0 {
            self.manual_brightness = value;
        }
        self.is_on = value > 0;
        self.brightness = value;
        self.persist();
    }

    /// Apply an auto-dim decision (transient: never updates the manual baseline or persists).
    /// `Some(v)` forces brightness `v` (0 = off); `None` restores the user's manual intent.
    pub fn apply_auto(&mut self, target: Option<u8>) {
        let had_effect = self.current_effect.is_some();
        self.stop_effect(); // an active rule takes over any running effect
        let (v, on) = resolve_auto(target, self.manual_on, self.manual_brightness);
        // Idempotent: skip the write when unchanged, unless an effect just ran (LED state unknown).
        if !had_effect && self.brightness == v && self.is_on == on {
            return;
        }
        if !self.write(v) {
            return;
        }
        self.is_on = on;
        self.brightness = v;
    }

    pub fn set_on(&mut self, on: bool) {
        self.stop_effect();
        if on {
            let v = if self.manual_brightness == 0 {
                0xff
            } else {
                self.manual_brightness
            };
            if !self.write(v) {
                return;
            }
            self.manual_on = true;
            self.manual_brightness = v;
            self.is_on = true;
            self.brightness = v;
        } else {
            if !self.write(0) {
                return;
            }
            self.manual_on = false;
            self.is_on = false;
            self.brightness = 0;
        }
        self.persist();
    }
}

pub static STATE: Mutex<Option<LedState>> = Mutex::new(None);

/// Watch for a crashed helper without waiting for an IPC response while holding
/// the global state lock. Commands still have a bounded response timeout.
/// A failed check records one error; the main-thread feedback timer presents it.
pub fn start_helper_monitor() {
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_secs(5));
        with_state(LedState::check_helper_health);
    });
}

pub fn with_state<R, F: FnOnce(&mut LedState) -> R>(f: F) -> Option<R> {
    // A poisoned lock is treated as a no-op by design (the `if let Ok(..)` is intentional).
    if let Ok(mut g) = STATE.lock() {
        if let Some(ref mut s) = *g {
            return Some(f(s));
        }
    }
    None
}

#[derive(Clone, Copy)]
pub struct LedStatus {
    pub is_on: bool,
    pub brightness: u8,
    pub helper_ready: bool,
}

pub fn read_status() -> LedStatus {
    // A poisoned lock is treated as a no-op by design (the `if let Ok(..)` is intentional).
    if let Ok(g) = STATE.lock() {
        if let Some(ref s) = *g {
            return LedStatus {
                is_on: s.is_on,
                brightness: s.brightness,
                helper_ready: s.helper_ready(),
            };
        }
    }
    LedStatus {
        is_on: false,
        brightness: 0,
        helper_ready: false,
    }
}

pub fn helper_available() -> bool {
    read_status().helper_ready
}

pub fn write_raw(value: u8, stop: &AtomicBool) {
    // A poisoned lock is treated as a no-op by design (the `if let Ok(..)` is intentional).
    if let Ok(mut g) = STATE.lock() {
        // A cancelled effect may already have passed its loop's stop check and
        // be waiting for this lock. Never let its stale write override a newer
        // manual or scheduled value after the lock is released.
        if stop.load(Ordering::Acquire) {
            return;
        }
        if let Some(ref mut s) = *g {
            s.write(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve_auto, Effect, LedAccess, LedState};
    use crate::helper::Helper;
    use std::io;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    fn state_without_helper() -> LedState {
        LedState {
            helper: None,
            control_error: Some("helper startup failed".into()),
            led_access: LedAccess::Ready,
            is_on: true,
            brightness: 0xff,
            manual_on: true,
            manual_brightness: 0xff,
            effect_stop: None,
            current_effect: None,
            auto_restart_used: false,
        }
    }

    #[test]
    fn auto_some_forces_the_value() {
        assert_eq!(resolve_auto(Some(0), true, 200), (0, false)); // night: off
        assert_eq!(resolve_auto(Some(51), true, 200), (51, true)); // night: dim
        assert_eq!(resolve_auto(Some(51), false, 200), (51, true)); // night rule ignores manual_on
    }

    #[test]
    fn missing_helper_is_not_ready() {
        assert!(!state_without_helper().helper_ready());
    }

    #[test]
    fn consumed_error_does_not_clear_failed_led_access() {
        let mut state = state_without_helper();
        assert!(!state.write(128));

        assert!(state.take_control_error().is_some());
        assert_eq!(state.led_access, LedAccess::WriteFailed);
        assert!(!state.helper_ready());
    }

    #[test]
    fn live_helper_with_failed_write_stays_degraded_after_feedback() -> io::Result<()> {
        let helper = Helper::spawn_test_echo()?;
        let mut state = LedState::with_helper(helper);
        assert!(state.helper_ready());

        state.set_on(false);
        assert!(state.take_control_error().is_some());
        assert!(!state.helper_ready());
        assert!(state.is_on);
        state.check_helper_health();
        assert!(!state.helper_ready());

        state
            .helper
            .as_mut()
            .ok_or_else(|| io::Error::other("test helper missing"))?
            .terminate()
    }

    #[cfg(feature = "field-test")]
    #[test]
    fn failed_field_test_helper_check_degrades_readiness() -> io::Result<()> {
        let helper = Helper::spawn_test_echo()?;
        let mut state = LedState::with_helper(helper);

        assert!(state.test_helper_connection().is_err());
        assert!(!state.helper_ready());

        state
            .helper
            .as_mut()
            .ok_or_else(|| io::Error::other("test helper missing"))?
            .terminate()
    }

    #[test]
    fn auto_none_restores_manual_intent() {
        // Daytime release restores the user's brightness when they want it on…
        assert_eq!(resolve_auto(None, true, 200), (200, true));
        // …and stays OFF when the user turned it off — never fights a manual OFF.
        assert_eq!(resolve_auto(None, false, 200), (0, false));
    }

    #[test]
    fn failed_write_does_not_fake_an_led_state_change() {
        let mut state = state_without_helper();
        state.set_on(false);

        assert!(state.is_on);
        assert_eq!(state.brightness, 0xff);
        assert_eq!(state.manual_brightness, 0xff);
        assert_eq!(
            state.take_control_error().as_deref(),
            Some("LED helper unavailable")
        );
    }

    #[test]
    fn failed_effect_preflight_does_not_start_an_effect() {
        let mut state = state_without_helper();
        state.start_effect(Effect::Pulse);

        assert!(state.current_effect.is_none());
        assert!(state.effect_stop.is_none());
        assert!(state.take_control_error().is_some());
    }

    #[test]
    fn failed_effect_write_stops_the_running_effect() {
        let mut state = state_without_helper();
        let stop = Arc::new(AtomicBool::new(false));
        state.effect_stop = Some(stop.clone());
        state.current_effect = Some(Effect::Pulse);

        assert!(!state.write(128));
        assert!(stop.load(Ordering::Relaxed));
        assert!(state.current_effect.is_none());
        assert!(state.take_control_error().is_some());
    }

    #[test]
    #[ignore = "requires a Mac mini with an approved profile and a root-owned setuid helper"]
    fn live_helper_restarts_once_after_crash() -> io::Result<()> {
        let helper = Helper::spawn()?;
        let mut state = LedState::with_helper(helper);
        let original = state
            .helper
            .as_mut()
            .ok_or_else(|| io::Error::other("helper missing before test"))?
            .read_profile_raw()?;
        if original != [255, 255] {
            return Err(io::Error::other(
                "hardware recovery test requires the LED to start at full brightness",
            ));
        }

        if let Some(helper) = state.helper.as_mut() {
            helper.terminate()?;
        }
        state.check_helper_health();
        assert!(state.helper_ready());
        assert!(state.auto_restart_used);
        assert!(state.take_control_error().is_none());
        assert_eq!(
            state
                .helper
                .as_mut()
                .ok_or_else(|| io::Error::other("helper missing after restart"))?
                .read_profile_raw()?,
            original
        );

        if let Some(helper) = state.helper.as_mut() {
            helper.terminate()?;
        }
        state.check_helper_health();
        assert!(!state.helper_ready());
        assert!(state.take_control_error().is_some());
        Ok(())
    }
}
