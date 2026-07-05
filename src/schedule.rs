//! Auto-dim scheduler: one level-triggered background thread that recomputes the
//! LED's target from (rules, local clock, sunrise/sunset) every tick and applies
//! it idempotently, so sleep/wake, DST and clock jumps self-heal. Brightness goes
//! through the transient `LedState::apply_auto`, never the user's manual baseline.

use crate::preferences;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The two auto-dim rules + their parameters. `enabled` is the master switch
/// (true iff at least one rule is on).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AutoDim {
    pub enabled: bool,
    /// Rule A: LED off from sunset until sunrise.
    pub off_at_sunset: bool,
    /// Rule B: dim to `dim_pct` from `dim_minute` until sunrise.
    pub dim_at_time: bool,
    /// Minutes after local midnight when the evening dim begins (default 21:00).
    pub dim_minute: u16,
    /// Evening dim level, 0..=100 % (default 20).
    pub dim_pct: u8,
}

impl Default for AutoDim {
    fn default() -> Self {
        Self {
            enabled: false,
            off_at_sunset: false,
            dim_at_time: false,
            dim_minute: 21 * 60,
            dim_pct: 20,
        }
    }
}

/// A geographic location (degrees; latitude +N, longitude +E).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Location {
    pub lat: f64,
    pub lon: f64,
}

#[allow(clippy::cast_possible_truncation)] // (pct*255+50)/100 with pct<=100 is in 0..=255
fn pct_to_byte(pct: u8) -> u8 {
    let p = u16::from(pct.min(100));
    ((p * 255 + 50) / 100) as u8
}

/// Is `now` inside a window from `start` to `sunrise`, wrapping past midnight?
/// With no sunrise (polar day) the window has no morning bound.
fn in_window(now: u16, start: u16, sunrise: Option<u16>) -> bool {
    match sunrise {
        Some(sr) => now >= start || now < sr,
        None => now >= start,
    }
}

/// The scheduler's target for `now` (local minutes-after-midnight): `Some(byte)`
/// forces that brightness (0 = off) for an active night rule, `None` = daytime
/// (caller restores the user's baseline).
#[must_use]
pub fn evaluate(d: AutoDim, now: u16, sunset: Option<u16>, sunrise: Option<u16>) -> Option<u8> {
    let mut target = None;
    // Rule B first…
    if d.dim_at_time && in_window(now, d.dim_minute, sunrise) {
        target = Some(pct_to_byte(d.dim_pct));
    }
    // …then Rule A, which wins when both fire (darker = stronger night intent).
    if d.off_at_sunset {
        if let Some(set) = sunset {
            if in_window(now, set, sunrise) {
                target = Some(0);
            }
        }
    }
    target
}

// ───────────────────────────── the scheduler thread ─────────────────────────

struct SchedHandle {
    stop: AtomicBool,
    wake: AtomicBool,
}

static SCHED: Mutex<Option<Arc<SchedHandle>>> = Mutex::new(None);
/// Epoch-seconds instant until which the scheduler stays passive after a manual
/// change (0 = no override). Lets the user touch the slider without a fight.
static MANUAL_OVERRIDE_UNTIL: AtomicI64 = AtomicI64::new(0);

const GRACE_SECS: i64 = 90 * 60;
const POLL_MS: u64 = 1_000;
const MAX_SLEEP_MS: u64 = 60_000;
const FALLBACK_LAT: f64 = 45.0;

/// Start (or restart) the single scheduler thread. Any prior thread is told to
/// stop first, so at most one ever exists — mirrors `start_effect`/`stop_effect`.
pub fn start() {
    let handle = Arc::new(SchedHandle {
        stop: AtomicBool::new(false),
        wake: AtomicBool::new(false),
    });
    {
        let Ok(mut g) = SCHED.lock() else { return };
        if let Some(old) = g.take() {
            old.stop.store(true, Ordering::Relaxed);
        }
        *g = Some(Arc::clone(&handle));
    }
    std::thread::spawn(move || run(&handle));
}

/// Stop the scheduler thread (the LED is left as-is — no surprise jump).
pub fn stop() {
    if let Ok(mut g) = SCHED.lock() {
        if let Some(old) = g.take() {
            old.stop.store(true, Ordering::Relaxed);
        }
    }
}

/// Start or stop according to the persisted master switch. Called after every
/// rule toggle (the handler saves prefs first).
pub fn restart() {
    if preferences::load_autodim().enabled {
        start();
    } else {
        stop();
    }
}

/// Record a manual user action: stay passive for `GRACE_SECS`, then reassert at
/// the next boundary. Also wakes the thread so it recomputes its sleep window.
pub fn note_manual_override() {
    MANUAL_OVERRIDE_UNTIL.store(epoch_now() + GRACE_SECS, Ordering::Relaxed);
    kick();
}

/// Nudge the thread to re-evaluate immediately (used on system wake).
pub fn kick() {
    if let Ok(g) = SCHED.lock() {
        if let Some(h) = g.as_ref() {
            h.wake.store(true, Ordering::Relaxed);
        }
    }
}

fn run(handle: &Arc<SchedHandle>) {
    while !handle.stop.load(Ordering::Relaxed) {
        evaluate_and_apply();
        // Cancellable sleep: poll stop/wake every second, cap at 60 s to self-heal a missed wake.
        let mut slept = 0u64;
        while slept < MAX_SLEEP_MS {
            if handle.stop.load(Ordering::Relaxed) {
                return;
            }
            if handle.wake.swap(false, Ordering::Relaxed) {
                break;
            }
            std::thread::sleep(Duration::from_millis(POLL_MS));
            slept += POLL_MS;
        }
    }
}

fn evaluate_and_apply() {
    // Manual-override grace: fully passive, never write.
    if epoch_now() < MANUAL_OVERRIDE_UNTIL.load(Ordering::Relaxed) {
        return;
    }
    let dim = preferences::load_autodim();
    if !dim.enabled {
        return;
    }
    let Some(now) = local_now() else { return };
    let loc = location(now.utc_offset_min);
    let st = crate::sun::sun_times(
        now.year,
        now.month,
        now.day,
        loc.lat,
        loc.lon,
        now.utc_offset_min,
    );
    let target = evaluate(dim, now.minute, st.sunset, st.sunrise);
    crate::led::with_state(|s| {
        // Re-check the override under the STATE lock so "user wins" is atomic vs a late manual change.
        if epoch_now() < MANUAL_OVERRIDE_UNTIL.load(Ordering::Relaxed) {
            return;
        }
        s.apply_auto(target);
    });
}

fn location(utc_offset_min: i32) -> Location {
    if let Some(loc) = preferences::load_location() {
        return loc;
    }
    // Zero-config fallback: 15°/h of longitude from the UTC offset, mid-northern latitude
    // (good to ~±30 min — invisible for an ambient dimmer).
    Location {
        lat: FALLBACK_LAT,
        lon: f64::from(utc_offset_min) / 60.0 * 15.0,
    }
}

// Local time via libc, declared inline to avoid the libc crate for two symbols.
// `localtime_r` is thread-safe and applies the system timezone (DST handled by the OS).

type TimeT = i64;

#[repr(C)]
#[allow(dead_code)] // FFI layout: several fields are unused but must exist for ABI correctness
#[allow(clippy::struct_field_names)] // field names mirror C `struct tm`'s ABI names
struct Tm {
    tm_sec: i32,
    tm_min: i32,
    tm_hour: i32,
    tm_mday: i32,
    tm_mon: i32,
    tm_year: i32,
    tm_wday: i32,
    tm_yday: i32,
    tm_isdst: i32,
    tm_gmtoff: i64,
    tm_zone: *const i8,
}

extern "C" {
    fn time(tloc: *mut TimeT) -> TimeT;
    fn localtime_r(clock: *const TimeT, result: *mut Tm) -> *mut Tm;
}

fn epoch_now() -> i64 {
    // SAFETY: time(NULL) returns the current Unix time and writes nothing.
    unsafe { time(std::ptr::null_mut()) }
}

struct LocalNow {
    year: i32,
    month: u32,
    day: u32,
    minute: u16,
    utc_offset_min: i32,
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // fields are bounded (month 1-12, day 1-31, minute 0-1439, offset ±840)
fn local_now() -> Option<LocalNow> {
    // SAFETY: localtime_r fills a caller-owned zeroed Tm; null-checked before reading fields.
    unsafe {
        let t = time(std::ptr::null_mut());
        let mut tm: Tm = std::mem::zeroed();
        if localtime_r(&raw const t, &raw mut tm).is_null() {
            return None;
        }
        Some(LocalNow {
            year: tm.tm_year + 1900,
            month: (tm.tm_mon + 1) as u32,
            day: tm.tm_mday as u32,
            minute: (tm.tm_hour * 60 + tm.tm_min) as u16,
            utc_offset_min: (tm.tm_gmtoff / 60) as i32,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{evaluate, pct_to_byte, AutoDim};

    fn dim(off_at_sunset: bool, dim_at_time: bool) -> AutoDim {
        AutoDim {
            enabled: true,
            off_at_sunset,
            dim_at_time,
            dim_minute: 21 * 60, // 1260
            dim_pct: 20,
        }
    }

    #[test]
    fn pct_to_byte_maps_endpoints() {
        assert_eq!(pct_to_byte(0), 0);
        assert_eq!(pct_to_byte(20), 51);
        assert_eq!(pct_to_byte(100), 255);
        assert_eq!(pct_to_byte(200), 255); // clamps
    }

    #[test]
    fn daytime_releases_control() {
        // 10:00, dim rule only, sunrise 07:00, sunset 21:30 → daytime → None.
        let d = dim(false, true);
        assert_eq!(evaluate(d, 600, Some(1290), Some(420)), None);
    }

    #[test]
    fn evening_dim_then_release_at_sunrise() {
        let d = dim(false, true);
        // 21:40 → dimmed.
        assert_eq!(evaluate(d, 1300, Some(1290), Some(420)), Some(51));
        // 08:00 (after sunrise) → released.
        assert_eq!(evaluate(d, 480, Some(1290), Some(420)), None);
        // 00:30 (still night, before sunrise) → dimmed.
        assert_eq!(evaluate(d, 30, Some(1290), Some(420)), Some(51));
    }

    #[test]
    fn off_at_sunset_window_wraps_midnight() {
        let d = dim(true, false);
        assert_eq!(evaluate(d, 1350, Some(1290), Some(420)), Some(0)); // 22:30
        assert_eq!(evaluate(d, 300, Some(1290), Some(420)), Some(0)); // 05:00 pre-sunrise
        assert_eq!(evaluate(d, 600, Some(1290), Some(420)), None); // 10:00 day
    }

    #[test]
    fn off_wins_over_dim_when_both_active() {
        let d = dim(true, true);
        // After sunset AND after dim time → OFF precedence.
        assert_eq!(evaluate(d, 1350, Some(1290), Some(420)), Some(0));
        // Evening, after dim (21:00) but before sunset (21:30) → just dimmed.
        assert_eq!(evaluate(d, 1270, Some(1290), Some(420)), Some(51));
    }

    #[test]
    fn polar_no_sunset_disables_off_rule() {
        // No sunset that day → the off-at-sunset rule no-ops (returns None).
        let d = dim(true, false);
        assert_eq!(evaluate(d, 600, None, None), None);
    }
}
