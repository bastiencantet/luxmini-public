//! Tiny FR/EN localization, detected once from the system's preferred languages
//! (French when the top preference starts with `fr`, English otherwise). Static
//! labels go through [`s`]; formatted strings branch on [`fr`].

use std::ffi::CStr;
use std::os::raw::c_char;
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Fr,
}

/// The process language, detected once and cached.
pub fn lang() -> Lang {
    static LANG: OnceLock<Lang> = OnceLock::new();
    *LANG.get_or_init(detect)
}

/// `true` when the UI should be in French.
pub fn fr() -> bool {
    lang() == Lang::Fr
}

/// Pick the English or French variant of a static label.
pub fn s(en: &'static str, fr: &'static str) -> &'static str {
    match lang() {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

/// Read `+[NSLocale preferredLanguages]` and map the top entry to a `Lang`.
fn detect() -> Lang {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};

    // SAFETY: +[NSLocale preferredLanguages] returns NSArray<NSString>*; pointers null-checked.
    unsafe {
        let Some(cls) = AnyClass::get(c"NSLocale") else {
            return Lang::En;
        };
        let langs: *mut AnyObject = msg_send![cls, preferredLanguages];
        if langs.is_null() {
            return Lang::En;
        }
        let count: usize = msg_send![langs, count];
        if count == 0 {
            return Lang::En;
        }
        let first: *mut AnyObject = msg_send![langs, objectAtIndex: 0usize];
        if first.is_null() {
            return Lang::En;
        }
        let utf8: *const c_char = msg_send![first, UTF8String];
        if utf8.is_null() {
            return Lang::En;
        }
        let code = CStr::from_ptr(utf8).to_string_lossy();
        if code.to_ascii_lowercase().starts_with("fr") {
            Lang::Fr
        } else {
            Lang::En
        }
    }
}
