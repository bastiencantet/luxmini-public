//! Sparkle auto-update: instantiate `SPUStandardUpdaterController` from the bundled
//! Sparkle.framework at startup. It reads `SUFeedURL`/`SUPublicEDKey` from Info.plist and
//! handles checks, download, signature verification, install and relaunch.

use std::sync::Mutex;

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};

// Sparkle links into the main tray binary only, not the setuid led-helper.
#[link(name = "Sparkle", kind = "framework")]
extern "C" {}

// Keep the controller alive for the app's lifetime; the Mutex only satisfies `Send`.
static UPDATER: Mutex<Option<Holder>> = Mutex::new(None);

struct Holder(Retained<AnyObject>);
#[allow(clippy::non_send_fields_in_send_ty)]
// SAFETY: both call sites run on the main thread (init from main.rs, check_for_updates from the
// AppKit handler); the Mutex guards ownership transfer only, never concurrent Obj-C messaging.
unsafe impl Send for Holder {}

/// Instantiate the Sparkle updater controller once at startup and keep it alive
/// for the app's lifetime (no-op if the framework class is unavailable).
pub fn init() {
    let Some(cls) = AnyClass::get(c"SPUStandardUpdaterController") else {
        eprintln!("sparkle: framework class not found (dev mode without bundle?)");
        return;
    };

    // SAFETY: alloc/init follow Cocoa +1 ownership (Retained::from_raw takes it); null handled below.
    let controller: Option<Retained<AnyObject>> = unsafe {
        let alloc: *mut AnyObject = objc2::msg_send![cls, alloc];
        if alloc.is_null() {
            eprintln!("sparkle: alloc returned nil");
            return;
        }
        let ctrl: *mut AnyObject = objc2::msg_send![
            alloc,
            initWithStartingUpdater: true,
            updaterDelegate: std::ptr::null_mut::<AnyObject>(),
            userDriverDelegate: std::ptr::null_mut::<AnyObject>(),
        ];
        Retained::from_raw(ctrl)
    };

    match controller {
        Some(c) => {
            if let Ok(mut g) = UPDATER.lock() {
                *g = Some(Holder(c));
            }
            eprintln!("sparkle: updater started");
        }
        None => eprintln!("sparkle: failed to init updater"),
    }
}

/// Trigger a visible update check (Sparkle's native "update available" / "up to date" UI).
pub fn check_for_updates() {
    let Ok(g) = UPDATER.lock() else { return };
    // Do NOT drop `g` early: the msg_send below borrows `holder` out of the guard.
    let Some(holder) = g.as_ref() else {
        eprintln!("sparkle: updater not initialized");
        return;
    };
    // SAFETY: holder.0 is the live controller; checkForUpdates: takes a sender id, main thread.
    unsafe {
        let _: () = objc2::msg_send![
            &*holder.0,
            checkForUpdates: std::ptr::null_mut::<AnyObject>(),
        ];
    }
}
