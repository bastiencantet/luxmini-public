//! LED state machine and the global STATE mutex; mediates all writes through the privileged helper.

use crate::effects::run_effect;
use crate::helper::Helper;
use crate::preferences;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Blink,
    BlinkFast,
    Pulse,
    Sos,
    Strobe,
}

pub struct LedState {
    helper: Option<Helper>,
    // LIVE state — what the LED shows now (the scheduler may dim/off it via `apply_auto`).
    is_on: bool,
    brightness: u8,
    // MANUAL intent — what the user last chose; the persisted day baseline the schedule
    // restores to. `apply_auto` never touches it, so a manual OFF is respected.
    manual_on: bool,
    manual_brightness: u8,
    effect_stop: Option<Arc<AtomicBool>>,
    current_effect: Option<Effect>,
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
        let helper = match Helper::spawn() {
            Ok(h) => Some(h),
            Err(e) => {
                eprintln!("helper not available: {e} (run `make setup`)");
                None
            }
        };
        Self {
            helper,
            is_on: true,
            brightness: 0xff,
            manual_on: true,
            manual_brightness: 0xff,
            effect_stop: None,
            current_effect: None,
        }
    }

    pub const fn with_helper(helper: Helper) -> Self {
        Self {
            helper: Some(helper),
            is_on: true,
            brightness: 0xff,
            manual_on: true,
            manual_brightness: 0xff,
            effect_stop: None,
            current_effect: None,
        }
    }

    fn write(&mut self, value: u8) {
        if let Some(ref mut h) = self.helper {
            if let Err(e) = h.write_led(value) {
                eprintln!("LED write failed: {e}");
            }
        }
    }

    fn stop_effect(&mut self) {
        if let Some(flag) = self.effect_stop.take() {
            flag.store(true, Ordering::Relaxed);
        }
        self.current_effect = None;
    }

    pub fn start_effect(&mut self, effect: Effect) {
        self.stop_effect();
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
        self.is_on = self.manual_on;
        self.brightness = v;
        self.write(v);
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

    #[allow(clippy::trivially_copy_pass_by_ref)] // mirrors preferences::save_*(&Preset) for call-site symmetry
    pub fn apply_preset(&mut self, p: &preferences::Preset) {
        self.stop_effect();
        self.manual_on = p.is_on;
        if p.brightness > 0 {
            self.manual_brightness = p.brightness;
        }
        let live = if p.is_on { p.brightness } else { 0 };
        self.is_on = p.is_on;
        self.brightness = live;
        if let Some(e) = p.effect {
            self.start_effect(e); // persists
        } else {
            self.write(live);
            self.persist();
        }
    }

    pub fn set_brightness(&mut self, value: u8) {
        self.stop_effect();
        self.manual_on = value > 0;
        if value > 0 {
            self.manual_brightness = value;
        }
        self.is_on = value > 0;
        self.brightness = value;
        self.write(value);
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
        self.is_on = on;
        self.brightness = v;
        self.write(v);
    }

    pub fn set_on(&mut self, on: bool) {
        self.stop_effect();
        self.manual_on = on;
        if on {
            let v = if self.manual_brightness == 0 {
                0xff
            } else {
                self.manual_brightness
            };
            self.manual_brightness = v;
            self.is_on = true;
            self.brightness = v;
            self.write(v);
        } else {
            self.is_on = false;
            self.brightness = 0;
            self.write(0);
        }
        self.persist();
    }
}

pub static STATE: Mutex<Option<LedState>> = Mutex::new(None);

pub fn with_state<F: FnOnce(&mut LedState)>(f: F) {
    // A poisoned lock is treated as a no-op by design (the `if let Ok(..)` is intentional).
    if let Ok(mut g) = STATE.lock() {
        if let Some(ref mut s) = *g {
            f(s);
        }
    }
}

pub fn read_state() -> (bool, u8) {
    // A poisoned lock is treated as a no-op by design (the `if let Ok(..)` is intentional).
    if let Ok(g) = STATE.lock() {
        if let Some(ref s) = *g {
            return (s.is_on, s.brightness);
        }
    }
    (false, 0)
}

pub fn write_raw(value: u8) {
    // A poisoned lock is treated as a no-op by design (the `if let Ok(..)` is intentional).
    if let Ok(mut g) = STATE.lock() {
        if let Some(ref mut s) = *g {
            if let Some(ref mut h) = s.helper {
                // best-effort; effect loop ignores transient write errors
                let _ = h.write_led(value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_auto;

    #[test]
    fn auto_some_forces_the_value() {
        assert_eq!(resolve_auto(Some(0), true, 200), (0, false)); // night: off
        assert_eq!(resolve_auto(Some(51), true, 200), (51, true)); // night: dim
        assert_eq!(resolve_auto(Some(51), false, 200), (51, true)); // night rule ignores manual_on
    }

    #[test]
    fn auto_none_restores_manual_intent() {
        // Daytime release restores the user's brightness when they want it on…
        assert_eq!(resolve_auto(None, true, 200), (200, true));
        // …and stays OFF when the user turned it off — never fights a manual OFF.
        assert_eq!(resolve_auto(None, false, 200), (0, false));
    }
}
