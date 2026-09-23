//! Sparkle auto-update: instantiate `SPUStandardUpdaterController` from the bundled
//! Sparkle.framework at startup. It reads `SUFeedURL`/`SUPublicEDKey` from Info.plist and
//! handles checks, download, signature verification, install and relaunch.

use std::sync::Mutex;

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_foundation::NSString;

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
    if !load_framework() {
        return;
    }
    let Some(cls) = AnyClass::get(c"SPUStandardUpdaterController") else {
        eprintln!("sparkle: updater class not found after framework load");
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

/// Load the signed framework before looking up its Objective-C classes. A link
/// declaration with no referenced symbols is stripped by the linker, leaving
/// the framework on disk but its classes unavailable at runtime.
fn load_framework() -> bool {
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("Frameworks/Sparkle.framework")));
    #[cfg(debug_assertions)]
    let bundled = bundled.filter(|path| path.is_dir()).or_else(|| {
        Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/Sparkle.framework"))
    });
    let Some(path) = bundled.filter(|path| path.is_dir()) else {
        eprintln!("sparkle: framework not found in the app bundle");
        return false;
    };
    let Some(bundle_class) = AnyClass::get(c"NSBundle") else {
        eprintln!("sparkle: NSBundle unavailable");
        return false;
    };
    let path = NSString::from_str(&path.to_string_lossy());
    // SAFETY: bundleWithPath: accepts an NSString path and returns a nullable
    // autoreleased NSBundle. load returns whether executable code was loaded.
    unsafe {
        let bundle: *mut AnyObject = objc2::msg_send![bundle_class, bundleWithPath: &*path];
        if bundle.is_null() {
            eprintln!("sparkle: framework bundle unavailable");
            return false;
        }
        let loaded: bool = objc2::msg_send![bundle, load];
        if !loaded {
            eprintln!("sparkle: framework could not be loaded");
        }
        loaded
    }
}

#[cfg(debug_assertions)]
pub fn framework_loads() -> bool {
    load_framework() && AnyClass::get(c"SPUStandardUpdaterController").is_some()
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
