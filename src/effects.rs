//! LED blink/pulse/strobe/SOS effect loops. Each runs on its own thread and polls an `AtomicBool` stop flag for prompt cancellation.

use crate::led::{write_raw, Effect};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn sleep_cancellable(ms: u64, stop: &AtomicBool) -> bool {
    let step_ms = 20;
    let mut remaining = ms;
    while remaining > 0 {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        let s = remaining.min(step_ms);
        std::thread::sleep(Duration::from_millis(s));
        // s = remaining.min(..) <= remaining, so this never underflows.
        remaining = remaining.saturating_sub(s);
    }
    !stop.load(Ordering::Relaxed)
}

#[allow(clippy::needless_pass_by_value)] // thread takes ownership of the stop flag
pub fn run_effect(effect: Effect, stop: Arc<AtomicBool>) {
    match effect {
        Effect::Blink => run_blink(500, &stop),
        Effect::BlinkFast => run_blink(120, &stop),
        Effect::Strobe => run_blink(45, &stop),
        Effect::Pulse => run_pulse(&stop),
        Effect::Sos => run_sos(&stop),
    }
}

fn run_blink(period_ms: u64, stop: &AtomicBool) {
    while !stop.load(Ordering::Relaxed) {
        write_raw(0xff);
        if !sleep_cancellable(period_ms, stop) {
            return;
        }
        write_raw(0x00);
        if !sleep_cancellable(period_ms, stop) {
            return;
        }
    }
}

fn run_pulse(stop: &AtomicBool) {
    let steps: u32 = 50;
    let step_ms: u64 = 40;
    let mut t: u32 = 0;
    while !stop.load(Ordering::Relaxed) {
        let phase = (f64::from(t) / f64::from(steps)) * std::f64::consts::TAU;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        // sin()*0.5+0.5 in [0,1], *255 in [0,255]
        let val = ((phase.sin() * 0.5 + 0.5) * 255.0).round() as u8;
        write_raw(val);
        std::thread::sleep(Duration::from_millis(step_ms));
        // t stays in 0..steps (< 50), so the add never actually wraps.
        t = t.wrapping_add(1) % steps;
    }
}

fn run_sos(stop: &AtomicBool) {
    let dot: u64 = 180;
    let dash = dot * 3;
    let gap = dot;
    let letter_gap = dot * 3;
    let word_gap = dot * 7;

    let pulse = |on_ms: u64, off_ms: u64, stop: &AtomicBool| -> bool {
        write_raw(0xff);
        if !sleep_cancellable(on_ms, stop) {
            return false;
        }
        write_raw(0x00);
        sleep_cancellable(off_ms, stop)
    };

    while !stop.load(Ordering::Relaxed) {
        for _ in 0..3 {
            if !pulse(dot, gap, stop) {
                return;
            }
        }
        if !sleep_cancellable(letter_gap.saturating_sub(gap), stop) {
            return;
        }
        for _ in 0..3 {
            if !pulse(dash, gap, stop) {
                return;
            }
        }
        if !sleep_cancellable(letter_gap.saturating_sub(gap), stop) {
            return;
        }
        for _ in 0..3 {
            if !pulse(dot, gap, stop) {
                return;
            }
        }
        if !sleep_cancellable(word_gap.saturating_sub(gap), stop) {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::sleep_cancellable;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn preset_flag_returns_false_without_sleeping() {
        let stop = AtomicBool::new(true);
        // A flag already set to stop returns false immediately (no sleep).
        assert!(!sleep_cancellable(10_000, &stop));
    }

    #[test]
    fn zero_ms_with_unset_flag_returns_true() {
        let stop = AtomicBool::new(false);
        // Zero duration never enters the loop; remaining stays 0, never underflows.
        assert!(sleep_cancellable(0, &stop));
        assert!(!stop.load(Ordering::Relaxed));
    }
}
