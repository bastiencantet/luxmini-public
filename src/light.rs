//! Shared light controller. Effects, schedules, presets and clients never inspect drivers.

use crate::effects::run_effect;
use crate::output::{self, DeviceInfo, LightKind, LightOutput, OutputStatus};
use crate::preferences;
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
enum LightAccess {
    Ready,
    WriteFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Persistence {
    Saved,
    SessionOnly,
}

pub struct LightState {
    output: Option<Box<dyn LightOutput>>,
    device: DeviceInfo,
    persistence: Persistence,
    control_error: Option<String>,
    access: LightAccess,
    is_on: bool,
    brightness: u8,
    manual_on: bool,
    manual_brightness: u8,
    effect_stop: Option<Arc<AtomicBool>>,
    current_effect: Option<Effect>,
}

const fn resolve_auto(target: Option<u8>, manual_on: bool, manual_brightness: u8) -> (u8, bool) {
    match target {
        Some(value) => (value, value > 0),
        None if manual_on => (manual_brightness, true),
        None => (0, false),
    }
}

impl LightState {
    pub fn open(device: DeviceInfo) -> Self {
        match output::open(device) {
            Ok(output) => Self::with_output(output, Persistence::Saved),
            Err(error) => Self::unavailable(device, &error),
        }
    }

    pub(crate) fn with_output(output: Box<dyn LightOutput>, persistence: Persistence) -> Self {
        let device = output.device();
        let value = output.initial_value();
        Self {
            device,
            output: Some(output),
            persistence,
            control_error: None,
            access: LightAccess::Ready,
            is_on: value.on,
            brightness: value.brightness,
            manual_on: value.on,
            manual_brightness: if value.brightness == 0 {
                255
            } else {
                value.brightness
            },
            effect_stop: None,
            current_effect: None,
        }
    }

    fn unavailable(device: DeviceInfo, error: &io::Error) -> Self {
        Self {
            device,
            output: None,
            persistence: Persistence::Saved,
            control_error: Some(error.to_string()),
            access: LightAccess::WriteFailed,
            is_on: false,
            brightness: 0,
            manual_on: false,
            manual_brightness: 255,
            effect_stop: None,
            current_effect: None,
        }
    }

    fn shutdown(&mut self) {
        self.stop_effect();
        if let Some(mut output) = self.output.take() {
            if let Err(error) = output.shutdown() {
                eprintln!("Light output shutdown failed: {error}");
            }
        }
    }

    pub fn is_ready(&self) -> bool {
        self.output.is_some() && self.access == LightAccess::Ready
    }

    pub fn check_health(&mut self) {
        let restore = if self.current_effect.is_some() {
            self.manual_brightness_if_on()
        } else {
            self.brightness
        };
        let Some(output) = self.output.as_mut() else {
            return;
        };
        match output.check_health(restore) {
            Ok(OutputStatus::Ready) => {} // liveness alone must not clear a rejected write
            result => {
                self.finish_output(result, restore);
            }
        }
    }

    #[cfg(feature = "field-test")]
    fn diagnose(&mut self, diagnostic: output::Diagnostic) -> io::Result<()> {
        let restore = self.manual_brightness_if_on();
        let result = self
            .output
            .as_mut()
            .ok_or_else(|| io::Error::other("Light output unavailable"))?
            .diagnose(diagnostic, restore);
        match result {
            Ok(status) => {
                self.finish_output(Ok(status), restore);
                Ok(())
            }
            Err(error) => {
                self.finish_output(
                    Err(io::Error::new(error.kind(), error.to_string())),
                    restore,
                );
                Err(error)
            }
        }
    }

    #[cfg(feature = "field-test")]
    pub fn test_output_connection(&mut self) -> io::Result<()> {
        self.diagnose(output::Diagnostic::Connection)
    }
    #[cfg(feature = "field-test")]
    pub fn test_output_restart(&mut self) -> io::Result<()> {
        self.diagnose(output::Diagnostic::Restart)
    }

    const fn manual_brightness_if_on(&self) -> u8 {
        if self.manual_on {
            self.manual_brightness
        } else {
            0
        }
    }

    fn write(&mut self, value: u8) -> bool {
        let result = self.output.as_mut().map_or_else(
            || Err(io::Error::other("Light output unavailable")),
            |output| output.write(value),
        );
        self.finish_output(result, value)
    }

    fn finish_output(&mut self, result: io::Result<OutputStatus>, value: u8) -> bool {
        match result {
            Ok(status) => {
                self.access = LightAccess::Ready;
                self.control_error = None;
                if status == OutputStatus::Reconnected && self.current_effect.is_some() {
                    self.stop_effect();
                    self.is_on = self.manual_on;
                    self.brightness = value;
                    self.persist();
                    self.control_error =
                        Some("Light connection recovered, but the active effect stopped".into());
                    return false;
                }
                true
            }
            Err(error) => {
                eprintln!("Light output failed: {error}");
                self.access = LightAccess::WriteFailed;
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

    fn apply_effect_frame(&mut self, value: u8, stop: &AtomicBool) {
        // Cancellation is rechecked while holding STATE, after any newer command.
        if !stop.load(Ordering::Acquire) {
            self.write(value);
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
        if self.persistence == Persistence::SessionOnly {
            return;
        }
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

pub static STATE: Mutex<Option<LightState>> = Mutex::new(None);

/// Run the active output's bounded health check without knowing its transport.
/// A failed check records one error; the main-thread feedback timer presents it.
pub fn start_output_monitor() {
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_secs(5));
        with_state(LightState::check_health);
    });
}

pub fn with_state<R, F: FnOnce(&mut LightState) -> R>(f: F) -> Option<R> {
    // A poisoned lock is treated as a no-op by design (the `if let Ok(..)` is intentional).
    if let Ok(mut g) = STATE.lock() {
        if let Some(ref mut s) = *g {
            return Some(f(s));
        }
    }
    None
}

#[derive(Clone, Copy)]
pub struct LightStatus {
    pub is_on: bool,
    pub brightness: u8,
    pub ready: bool,
    pub device: DeviceInfo,
}

pub fn read_status() -> LightStatus {
    // A poisoned lock is treated as a no-op by design (the `if let Ok(..)` is intentional).
    if let Ok(g) = STATE.lock() {
        if let Some(ref s) = *g {
            return LightStatus {
                is_on: s.is_on,
                brightness: s.brightness,
                ready: s.is_ready(),
                device: s.device,
            };
        }
    }
    LightStatus {
        is_on: false,
        brightness: 0,
        ready: false,
        device: DeviceInfo::physical(LightKind::FrontLed),
    }
}

pub fn output_available() -> bool {
    read_status().ready
}

pub fn write_raw(value: u8, stop: &AtomicBool) {
    // A poisoned lock is treated as a no-op by design (the `if let Ok(..)` is intentional).
    if let Ok(mut g) = STATE.lock() {
        // A cancelled effect may already have passed its loop's stop check and
        // be waiting for this lock. Never let its stale write override a newer
        // manual or scheduled value after the lock is released.
        if let Some(ref mut state) = *g {
            state.apply_effect_frame(value, stop);
        }
    }
}

/// Stop effects and release the active output through its own shutdown contract.
pub fn shutdown() {
    with_state(LightState::shutdown);
}

impl Drop for LightState {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::LightValue;
    use std::sync::atomic::{AtomicU8, AtomicUsize};

    struct Probe {
        device: DeviceInfo,
        writes: Arc<Mutex<Vec<u8>>>,
        reject: Arc<AtomicBool>,
        recovered: Arc<AtomicBool>,
        shutdowns: Arc<AtomicUsize>,
    }

    impl LightOutput for Probe {
        fn device(&self) -> DeviceInfo {
            self.device
        }
        fn initial_value(&self) -> LightValue {
            LightValue {
                on: true,
                brightness: 255,
            }
        }
        fn write(&mut self, value: u8) -> io::Result<OutputStatus> {
            if self.reject.swap(false, Ordering::Relaxed) {
                return Err(io::Error::other("rejected"));
            }
            self.writes
                .lock()
                .map_err(|_| io::Error::other("probe lock"))?
                .push(value);
            Ok(OutputStatus::Ready)
        }
        fn check_health(&mut self, _restore: u8) -> io::Result<OutputStatus> {
            Ok(if self.recovered.swap(false, Ordering::Relaxed) {
                OutputStatus::Reconnected
            } else {
                OutputStatus::Ready
            })
        }
        fn shutdown(&mut self) -> io::Result<()> {
            self.shutdowns.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    fn probe(kind: LightKind) -> Probe {
        Probe {
            device: DeviceInfo::physical(kind),
            writes: Arc::default(),
            reject: Arc::default(),
            recovered: Arc::default(),
            shutdowns: Arc::default(),
        }
    }

    /// Every new shared command belongs in this contract suite for both devices.
    #[test]
    fn both_devices_share_manual_presets_and_scheduling() -> io::Result<()> {
        let mut traces = Vec::new();
        for kind in [LightKind::FrontLed, LightKind::RearLogo] {
            let output = probe(kind);
            let writes = output.writes.clone();
            let mut state = LightState::with_output(Box::new(output), Persistence::SessionOnly);
            assert_eq!(state.device.kind, kind);
            state.set_brightness(128);
            let saved = state.capture_preset();
            state.set_on(false);
            state.apply_auto(None);
            assert!(!state.is_on); // scheduler respects a manual off
            state.apply_auto(Some(51));
            assert!(state.is_on);
            state.apply_auto(None);
            assert!(!state.is_on);
            state.apply_preset(&saved);
            assert_eq!(state.capture_preset().brightness, 128);
            state.apply_auto(Some(0));
            assert_eq!(state.brightness, 0);
            state.apply_auto(None);
            assert_eq!(state.brightness, 128);
            state.set_on(false);
            state.set_on(true);
            assert_eq!(state.brightness, 128);
            traces.push(
                writes
                    .lock()
                    .map_err(|_| io::Error::other("probe lock"))?
                    .clone(),
            );
        }
        assert_eq!(traces.first(), traces.last());
        assert_eq!(
            traces.first(),
            Some(&vec![128, 0, 51, 0, 128, 0, 128, 0, 128])
        );
        Ok(())
    }

    #[test]
    fn both_devices_keep_failed_writes_out_of_confirmed_state() {
        for kind in [LightKind::FrontLed, LightKind::RearLogo] {
            let output = probe(kind);
            let reject = output.reject.clone();
            let mut state = LightState::with_output(Box::new(output), Persistence::SessionOnly);
            reject.store(true, Ordering::Relaxed);
            state.set_on(false);
            assert!(state.is_on);
            assert_eq!(state.brightness, 255);
            assert!(!state.is_ready());
            assert!(state.take_control_error().is_some());
            state.check_health();
            assert!(!state.is_ready()); // liveness does not erase a rejected command
            state.set_brightness(100);
            assert!(state.is_ready());
        }
    }

    #[test]
    fn both_devices_stop_effects_on_error_and_restore_manual_intent_on_reconnect() {
        for kind in [LightKind::FrontLed, LightKind::RearLogo] {
            let output = probe(kind);
            let reject = output.reject.clone();
            let recovered = output.recovered.clone();
            let mut state = LightState::with_output(Box::new(output), Persistence::SessionOnly);
            state.set_brightness(90);
            let stop = Arc::new(AtomicBool::new(false));
            state.current_effect = Some(Effect::Pulse);
            state.effect_stop = Some(stop.clone());
            recovered.store(true, Ordering::Relaxed);
            state.check_health();
            assert!(stop.load(Ordering::Relaxed));
            assert!(state.current_effect.is_none());
            assert_eq!(state.brightness, 90);
            assert!(state.is_ready());
            assert!(state.take_control_error().is_some());
            state.current_effect = Some(Effect::Blink);
            state.effect_stop = Some(Arc::new(AtomicBool::new(false)));
            reject.store(true, Ordering::Relaxed);
            assert!(!state.write(128));
            assert!(state.current_effect.is_none());
            assert!(!state.is_ready());
        }
    }

    #[test]
    fn all_effects_share_frame_output_and_cancel_for_both_devices() -> io::Result<()> {
        for kind in [LightKind::FrontLed, LightKind::RearLogo] {
            for effect in [
                Effect::Blink,
                Effect::BlinkFast,
                Effect::Pulse,
                Effect::Sos,
                Effect::Strobe,
            ] {
                let output = probe(kind);
                let writes = output.writes.clone();
                let mut state = LightState::with_output(Box::new(output), Persistence::SessionOnly);
                state.set_brightness(90);
                let stop = Arc::new(AtomicBool::new(false));
                state.effect_stop = Some(stop.clone());
                state.current_effect = Some(effect);
                for value in [0, 128, 255] {
                    state.apply_effect_frame(value, &stop);
                }
                state.clear_effect();
                state.apply_effect_frame(255, &stop); // stale frame cannot override manual intent
                assert!(stop.load(Ordering::Acquire));
                assert_eq!(state.brightness, 90);
                assert!(state.current_effect.is_none());
                assert_eq!(
                    *writes.lock().map_err(|_| io::Error::other("probe lock"))?,
                    vec![90, 0, 128, 255, 90]
                );
            }
        }
        Ok(())
    }

    #[test]
    fn both_devices_release_output_exactly_once() {
        for kind in [LightKind::FrontLed, LightKind::RearLogo] {
            let output = probe(kind);
            let shutdowns = output.shutdowns.clone();
            {
                let mut state = LightState::with_output(Box::new(output), Persistence::SessionOnly);
                state.shutdown();
                assert!(!state.is_ready());
                assert!(!state.write(200));
            }
            assert_eq!(shutdowns.load(Ordering::Relaxed), 1);
        }
    }

    #[test]
    fn memory_driver_supports_both_presentations_without_persistence() {
        for kind in [LightKind::FrontLed, LightKind::RearLogo] {
            let value = Arc::new(AtomicU8::new(0));
            let mut state = LightState::with_output(
                output::simulated(kind, value.clone()),
                Persistence::SessionOnly,
            );
            state.set_brightness(128);
            state.set_on(false);
            assert_eq!(value.load(Ordering::Relaxed), 0);
            state.set_on(true);
            assert_eq!(value.load(Ordering::Relaxed), 128);
            assert!(state.device.simulated);
        }
    }

    #[test]
    fn unavailable_driver_does_not_fake_a_state_transition() {
        for kind in [LightKind::FrontLed, LightKind::RearLogo] {
            let mut state = LightState::unavailable(
                DeviceInfo::physical(kind),
                &io::Error::other("startup failed"),
            );
            state.set_on(true);
            assert!(!state.is_ready());
            assert!(!state.is_on);
            assert_eq!(state.brightness, 0);
            state.start_effect(Effect::Pulse);
            assert!(state.current_effect.is_none());
            assert!(state.take_control_error().is_some());
        }
    }

    #[test]
    fn auto_rule_respects_manual_intent() {
        assert_eq!(resolve_auto(Some(51), false, 200), (51, true));
        assert_eq!(resolve_auto(None, true, 200), (200, true));
        assert_eq!(resolve_auto(None, false, 200), (0, false));
    }

    #[cfg(feature = "field-test")]
    #[test]
    fn unsupported_diagnostics_fail_without_hardware_fallback() {
        let mut state = LightState::with_output(
            output::simulated(LightKind::RearLogo, Arc::default()),
            Persistence::SessionOnly,
        );
        assert!(state.test_output_connection().is_err());
        assert!(!state.is_ready());
    }
}
