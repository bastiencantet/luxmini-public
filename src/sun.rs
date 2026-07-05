//! Offline sunrise/sunset — the NOAA low-precision solar-position algorithm in pure
//! `std` f64 (no crates, network, or FFI): a date + location yields the day's sunrise
//! and sunset as local minutes-after-midnight, so auto-dim works fully offline.

use std::f64::consts::PI;

/// Official sunrise/sunset zenith (degrees): 90° + refraction + solar-disk radius.
const ZENITH_DEG: f64 = 90.833;

/// Sunrise and sunset for one day, as local minutes-after-midnight (`0..=1439`).
/// `None` for a leg means the event does not occur that day (polar day/night).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SunTimes {
    pub sunrise: Option<u16>,
    pub sunset: Option<u16>,
}

const fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// 1-based day of the year. An out-of-range month is treated as January, which
/// keeps the function total (callers pass real months from `localtime`).
fn day_of_year(year: i32, month: u32, day: u32) -> u32 {
    // Days before the 1st of `month` (non-leap year); match avoids indexing.
    let before = match month {
        2 => 31,
        3 => 59,
        4 => 90,
        5 => 120,
        6 => 151,
        7 => 181,
        8 => 212,
        9 => 243,
        10 => 273,
        11 => 304,
        12 => 334,
        _ => 0,
    };
    let leap_bump = u32::from(is_leap(year) && month > 2);
    before + leap_bump + day
}

/// Wrap a (possibly negative or >1440) minute value into `0..=1439` and round.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // wrapped into 0..1440 before the cast
fn to_local_minute(utc_min: f64, utc_offset_min: i32) -> u16 {
    let local = utc_min + f64::from(utc_offset_min);
    // rem_euclid keeps the result in [0, 1440); round may land on 1440.0, so wrap once more.
    (local.rem_euclid(1440.0).round()).rem_euclid(1440.0) as u16
}

/// Sunrise/sunset for the given local date and location.
///
/// `lat_deg` is positive north, `lon_east_deg` positive **east** (geographic
/// convention); `utc_offset_min` is the local zone's offset from UTC in minutes
/// for that date (DST included). Returns local minutes-after-midnight.
#[allow(clippy::suboptimal_flops)] // readability of the NOAA series > mul_add micro-optimisation
#[must_use]
pub fn sun_times(
    year: i32,
    month: u32,
    day: u32,
    lat_deg: f64,
    lon_east_deg: f64,
    utc_offset_min: i32,
) -> SunTimes {
    let n = day_of_year(year, month, day);
    let days_in_year = if is_leap(year) { 366.0 } else { 365.0 };
    // Fractional year (radians), evaluated at solar noon (the (hour-12)/24 term is 0).
    let gamma = (2.0 * PI / days_in_year) * (f64::from(n) - 1.0);

    // Equation of time (minutes) and solar declination (radians) — NOAA Fourier series.
    let eqtime = 229.18
        * (0.000_075 + 0.001_868 * gamma.cos()
            - 0.032_077 * gamma.sin()
            - 0.014_615 * (2.0 * gamma).cos()
            - 0.040_849 * (2.0 * gamma).sin());
    let decl = 0.006_918 - 0.399_912 * gamma.cos() + 0.070_257 * gamma.sin()
        - 0.006_758 * (2.0 * gamma).cos()
        + 0.000_907 * (2.0 * gamma).sin()
        - 0.002_697 * (3.0 * gamma).cos()
        + 0.001_480 * (3.0 * gamma).sin();

    let lat = lat_deg.to_radians();
    let zenith = ZENITH_DEG.to_radians();
    // cos(hour angle) at sunrise/sunset. A |value| > 1 means the sun stays below
    // (polar night) or above (polar day) the horizon all day → no event.
    let cos_ha = (zenith.cos() / (lat.cos() * decl.cos())) - lat.tan() * decl.tan();
    if !(-1.0..=1.0).contains(&cos_ha) {
        return SunTimes {
            sunrise: None,
            sunset: None,
        };
    }
    let ha_deg = cos_ha.acos().to_degrees();
    // East-positive UTC form: 720 - 4*(lon_east ± ha) - eqtime. lon_east goes straight in
    // (a west-flip here would double-negate it — past regression).
    let sunrise_utc = 720.0 - 4.0 * (lon_east_deg + ha_deg) - eqtime;
    let sunset_utc = 720.0 - 4.0 * (lon_east_deg - ha_deg) - eqtime;
    SunTimes {
        sunrise: Some(to_local_minute(sunrise_utc, utc_offset_min)),
        sunset: Some(to_local_minute(sunset_utc, utc_offset_min)),
    }
}

#[cfg(test)]
mod tests {
    use super::{day_of_year, sun_times};

    #[test]
    fn day_of_year_handles_leap() {
        assert_eq!(day_of_year(2026, 1, 1), 1);
        assert_eq!(day_of_year(2026, 3, 1), 60); // 2026 not leap: 31+28+1
        assert_eq!(day_of_year(2024, 3, 1), 61); // 2024 leap: 31+29+1
        assert_eq!(day_of_year(2026, 12, 31), 365);
        assert_eq!(day_of_year(2024, 12, 31), 366);
    }

    #[test]
    fn paris_summer_solstice() {
        // Paris (48.85N, 2.35E), 2026-06-21, CEST = UTC+2 (120 min).
        // Almanac: sunrise ~05:46 (346), sunset ~21:57 (1317). Tight windows so a
        // longitude/sign regression (≈19 min off here) is caught.
        let st = sun_times(2026, 6, 21, 48.8566, 2.3522, 120);
        assert!(
            matches!(st.sunrise, Some(m) if (330..=360).contains(&m)),
            "sunrise {:?} not ~05:46",
            st.sunrise
        );
        assert!(
            matches!(st.sunset, Some(m) if (1302..=1332).contains(&m)),
            "sunset {:?} not ~21:57",
            st.sunset
        );
        assert!(matches!(
            (st.sunrise, st.sunset),
            (Some(sr), Some(ss)) if ss - sr > 900 // > 15 h summer day
        ));
    }

    #[test]
    fn new_york_summer_western_hemisphere() {
        // Western hemisphere regression guard for the longitude sign.
        // NYC (40.71N, 74.0W → lon -74E), 2026-06-21, EDT = UTC-4 (-240 min).
        // Almanac: sunrise ~05:24 (324), sunset ~20:30 (1230). The old sign bug put
        // sunrise *after* sunset (~10 h off).
        let st = sun_times(2026, 6, 21, 40.7128, -74.0, -240);
        assert!(
            matches!(st.sunrise, Some(m) if (305..=345).contains(&m)),
            "sunrise {:?} not ~05:24",
            st.sunrise
        );
        assert!(
            matches!(st.sunset, Some(m) if (1210..=1250).contains(&m)),
            "sunset {:?} not ~20:30",
            st.sunset
        );
        assert!(matches!(
            (st.sunrise, st.sunset),
            (Some(sr), Some(ss)) if sr < ss // sunrise before sunset (sign sanity)
        ));
    }

    #[test]
    fn equator_equinox_is_about_twelve_hours() {
        // Equator, March equinox: day length ~12 h regardless of longitude.
        let st = sun_times(2026, 3, 20, 0.0, 0.0, 0);
        assert!(matches!(
            (st.sunrise, st.sunset),
            (Some(sr), Some(ss)) if (690..=750).contains(&(ss - sr))
        ));
    }

    #[test]
    fn polar_summer_has_no_sunset() {
        // 78N at the June solstice: midnight sun → no sunset (and no sunrise).
        let st = sun_times(2026, 6, 21, 78.0, 15.0, 60);
        assert_eq!(st.sunset, None);
        assert_eq!(st.sunrise, None);
    }

    #[test]
    fn polar_winter_has_no_sunrise() {
        // 78N at the December solstice: polar night → no sunrise.
        let st = sun_times(2026, 12, 21, 78.0, 15.0, 60);
        assert_eq!(st.sunrise, None);
        assert_eq!(st.sunset, None);
    }
}
