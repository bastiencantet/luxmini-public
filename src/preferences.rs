//! Preset storage via `NSUserDefaults`. Each slot stores the brightness,
//! on/off, and the currently running effect (if any).

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_foundation::NSString;

use crate::led::Effect;
use crate::schedule::{AutoDim, Location};

#[derive(Clone, Copy, Debug)]
pub struct Preset {
    pub brightness: u8,
    pub is_on: bool,
    pub effect: Option<Effect>,
}

pub fn load_preset(slot: u8) -> Option<Preset> {
    let prefix = format!("preset.{slot}");
    if !get_bool(&format!("{prefix}.set")) {
        return None;
    }
    let brightness = clamp_brightness(get_int(&format!("{prefix}.brightness")));
    let is_on = get_bool(&format!("{prefix}.is_on"));
    let effect = get_str(&format!("{prefix}.effect"))
        .as_deref()
        .and_then(effect_from_str);
    Some(Preset {
        brightness,
        is_on,
        effect,
    })
}

#[allow(clippy::trivially_copy_pass_by_ref)] // mirrors led.rs apply_preset(&Preset) for call-site symmetry
pub fn save_preset(slot: u8, p: &Preset) {
    let prefix = format!("preset.{slot}");
    set_bool(&format!("{prefix}.set"), true);
    set_int(&format!("{prefix}.brightness"), i64::from(p.brightness));
    set_bool(&format!("{prefix}.is_on"), p.is_on);
    set_str(&format!("{prefix}.effect"), p.effect.map(effect_to_str));
}

// "Last session" state — persisted on every change so a quit/logout/reboot
// resumes the LED exactly where it left off.

pub fn load_last_state() -> Option<Preset> {
    if !get_bool("state.set") {
        return None;
    }
    let brightness = clamp_brightness(get_int("state.brightness"));
    let is_on = get_bool("state.is_on");
    let effect = get_str("state.effect").as_deref().and_then(effect_from_str);
    Some(Preset {
        brightness,
        is_on,
        effect,
    })
}

#[allow(clippy::trivially_copy_pass_by_ref)] // mirrors led.rs apply_preset(&Preset) for call-site symmetry
pub fn save_last_state(p: &Preset) {
    set_bool("state.set", true);
    set_int("state.brightness", i64::from(p.brightness));
    set_bool("state.is_on", p.is_on);
    set_str("state.effect", p.effect.map(effect_to_str));
}

// Auto-dim schedule — the rules + (optional) precise location.

pub fn load_autodim() -> AutoDim {
    // Never configured → sensible defaults (disabled, 21:00, 20%).
    if !get_bool("autodim.set") {
        return AutoDim::default();
    }
    AutoDim {
        enabled: get_bool("autodim.enabled"),
        off_at_sunset: get_bool("autodim.off_at_sunset"),
        dim_at_time: get_bool("autodim.dim_at_time"),
        dim_minute: clamp_minute(get_int("autodim.dim_minute")),
        dim_pct: clamp_pct(get_int("autodim.dim_pct")),
    }
}

#[allow(clippy::trivially_copy_pass_by_ref)] // mirrors the other save_* signatures for call-site symmetry
pub fn save_autodim(d: &AutoDim) {
    set_bool("autodim.set", true);
    set_bool("autodim.enabled", d.enabled);
    set_bool("autodim.off_at_sunset", d.off_at_sunset);
    set_bool("autodim.dim_at_time", d.dim_at_time);
    set_int("autodim.dim_minute", i64::from(d.dim_minute));
    set_int("autodim.dim_pct", i64::from(d.dim_pct));
}

/// A precise manual location, if the user has set one (else `None` → the
/// scheduler derives an approximate location from the system timezone).
pub fn load_location() -> Option<Location> {
    if !get_bool("autodim.loc.set") {
        return None;
    }
    let lat = get_str("autodim.loc.lat")?.parse::<f64>().ok()?;
    let lon = get_str("autodim.loc.lon")?.parse::<f64>().ok()?;
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    Some(Location { lat, lon })
}

/// Store a precise manual location (degrees). Used by the Settings window.
pub fn save_location(lat: f64, lon: f64) {
    set_bool("autodim.loc.set", true);
    set_str("autodim.loc.lat", Some(&format!("{lat}")));
    set_str("autodim.loc.lon", Some(&format!("{lon}")));
}

/// Forget the manual location → the scheduler falls back to the timezone-derived
/// (approximate) one.
pub fn clear_location() {
    set_bool("autodim.loc.set", false);
}

/// Whether the menu-bar icon should be hidden (set-and-forget mode).
pub fn load_hide_icon() -> bool {
    get_bool("ui.hide_icon")
}

pub fn save_hide_icon(hidden: bool) {
    set_bool("ui.hide_icon", hidden);
}

/// Local control API (see `crate::api`) — opt-in, off by default, localhost-only.
/// Toggled from Settings › General; also settable via
/// `defaults write com.bastiencantet.LuxMini api.enabled -bool true`.
pub fn api_enabled() -> bool {
    get_bool("api.enabled")
}

pub fn save_api_enabled(enabled: bool) {
    set_bool("api.enabled", enabled);
}

/// Port for the local API. Defaults to 4470 when unset or out of range.
pub fn api_port() -> u16 {
    u16::try_from(get_int("api.port"))
        .ok()
        .filter(|&p| p != 0)
        .unwrap_or(4470)
}

/// Optional bearer token gating the local API. `None` → no token (localhost is
/// the trust boundary); set one to require `Authorization: Bearer <token>`.
pub fn api_token() -> Option<String> {
    get_str("api.token").filter(|s| !s.is_empty())
}

/// Saturating `i64` → `u8` brightness clamp. Manual (keeps it `const`); the cast is lossless.
const fn clamp_brightness(v: i64) -> u8 {
    if v <= 0 {
        0
    } else if v >= 255 {
        255
    } else {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // narrowed to 1..=254
        let b = v as u8;
        b
    }
}

/// Saturating `i64` → `u8` clamp to 0..=100 %. Manual (keeps it `const`); cast is lossless.
const fn clamp_pct(v: i64) -> u8 {
    if v <= 0 {
        0
    } else if v >= 100 {
        100
    } else {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // narrowed to 1..=99
        let p = v as u8;
        p
    }
}

/// Saturating clamp of a stored `i64` to a 0..=1439 minute-of-day.
const fn clamp_minute(v: i64) -> u16 {
    if v <= 0 {
        0
    } else if v >= 1439 {
        1439
    } else {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // narrowed to 1..=1438
        let m = v as u16;
        m
    }
}

const fn effect_to_str(e: Effect) -> &'static str {
    match e {
        Effect::Blink => "blink",
        Effect::BlinkFast => "blink_fast",
        Effect::Pulse => "pulse",
        Effect::Sos => "sos",
        Effect::Strobe => "strobe",
    }
}

fn effect_from_str(s: &str) -> Option<Effect> {
    match s {
        "blink" => Some(Effect::Blink),
        "blink_fast" => Some(Effect::BlinkFast),
        "pulse" => Some(Effect::Pulse),
        "sos" => Some(Effect::Sos),
        "strobe" => Some(Effect::Strobe),
        _ => None,
    }
}

fn defaults() -> Option<*mut AnyObject> {
    // SAFETY: standardUserDefaults returns the shared singleton id (or nil, checked).
    unsafe {
        let cls = AnyClass::get(c"NSUserDefaults")?;
        let defaults: *mut AnyObject = objc2::msg_send![cls, standardUserDefaults];
        if defaults.is_null() {
            None
        } else {
            Some(defaults)
        }
    }
}

fn set_int(key: &str, value: i64) {
    let Some(defaults) = defaults() else { return };
    // SAFETY: setInteger:forKey: takes an NSInteger + non-null NSString*, returns void.
    unsafe {
        let k = NSString::from_str(key);
        let _: () = objc2::msg_send![defaults, setInteger: value, forKey: &*k];
    }
}

fn set_bool(key: &str, value: bool) {
    let Some(defaults) = defaults() else { return };
    // SAFETY: setBool:forKey: takes a BOOL and a non-null NSString*; both arguments match the
    // selector signature and the call returns void.
    unsafe {
        let k = NSString::from_str(key);
        let _: () = objc2::msg_send![defaults, setBool: value, forKey: &*k];
    }
}

fn set_str(key: &str, value: Option<&str>) {
    let Some(defaults) = defaults() else { return };
    // SAFETY: setObject:forKey: / removeObjectForKey: take non-null NSString* args matching the
    // selector signatures and return void; the temporaries outlive the calls.
    unsafe {
        let k = NSString::from_str(key);
        match value {
            Some(v) => {
                let vs = NSString::from_str(v);
                let _: () = objc2::msg_send![defaults, setObject: &*vs, forKey: &*k];
            }
            None => {
                let _: () = objc2::msg_send![defaults, removeObjectForKey: &*k];
            }
        }
    }
}

fn get_int(key: &str) -> i64 {
    let Some(defaults) = defaults() else { return 0 };
    // SAFETY: integerForKey: takes a non-null NSString* and returns an NSInteger (i64 here).
    unsafe {
        let k = NSString::from_str(key);
        objc2::msg_send![defaults, integerForKey: &*k]
    }
}

fn get_bool(key: &str) -> bool {
    let Some(defaults) = defaults() else {
        return false;
    };
    // SAFETY: boolForKey: takes a non-null NSString* and returns a BOOL.
    unsafe {
        let k = NSString::from_str(key);
        objc2::msg_send![defaults, boolForKey: &*k]
    }
}

fn get_str(key: &str) -> Option<String> {
    let defaults = defaults()?;
    // SAFETY: stringForKey: returns an autoreleased NSString* or nil; Retained::retain handles
    // nil via Option and adds our own +1 so the value outlives the autorelease pool.
    unsafe {
        let k = NSString::from_str(key);
        let ptr: *mut NSString = objc2::msg_send![defaults, stringForKey: &*k];
        let retained = Retained::retain(ptr)?;
        Some(retained.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{clamp_brightness, effect_from_str, effect_to_str};
    use crate::led::Effect;

    #[test]
    fn effect_str_round_trip() {
        for e in [
            Effect::Blink,
            Effect::BlinkFast,
            Effect::Pulse,
            Effect::Sos,
            Effect::Strobe,
        ] {
            assert_eq!(effect_from_str(effect_to_str(e)), Some(e));
        }
    }

    #[test]
    fn effect_from_str_rejects_unknown() {
        assert_eq!(effect_from_str("nonsense"), None);
    }

    #[test]
    fn clamp_brightness_saturates() {
        assert_eq!(clamp_brightness(-10), 0);
        assert_eq!(clamp_brightness(0), 0);
        assert_eq!(clamp_brightness(200), 200);
        assert_eq!(clamp_brightness(255), 255);
        assert_eq!(clamp_brightness(1000), 255);
    }

    #[test]
    fn clamp_brightness_boundaries() {
        // Pin the edges of the >=255 branch and the 1..=254 pass-through.
        assert_eq!(clamp_brightness(254), 254);
        assert_eq!(clamp_brightness(256), 255);
    }
}
