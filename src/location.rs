//! `CoreLocation` integration: a one-shot location fix (with permission) to fill the
//! auto-dim location automatically. ~city accuracy is enough for sunrise/sunset and the
//! coordinate never leaves the Mac. Requires `NSLocationWhenInUseUsageDescription` in
//! Info.plist. `Handler` is the `CLLocationManager` delegate; the manager is kept alive
//! in its ivars for the async request.

use crate::preferences;
use crate::schedule;
use crate::ui::handler::Handler;
use crate::ui::settings;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{msg_send, DefinedClass, Encode, Encoding};
use objc2_foundation::NSObject;

// CoreLocation links into the main tray binary only, not the setuid led-helper.
#[link(name = "CoreLocation", kind = "framework")]
extern "C" {}

// CLAuthorizationStatus values (CoreLocation). NotDetermined (0) is the default
// catch-all branch (prompt still on screen), so it needs no named constant.
const RESTRICTED: i32 = 1;
const DENIED: i32 = 2;
const AUTHORIZED_ALWAYS: i32 = 3;
const AUTHORIZED_WHEN_IN_USE: i32 = 4;

/// `CoreLocation`'s `CLLocationCoordinate2D`: two `CLLocationDegrees` (C double).
#[repr(C)]
#[derive(Clone, Copy)]
struct Coordinate {
    latitude: f64,
    longitude: f64,
}

// SAFETY: mirrors CLLocationCoordinate2D: two doubles in declaration order.
unsafe impl Encode for Coordinate {
    const ENCODING: Encoding = Encoding::Struct(
        "CLLocationCoordinate2D",
        &[Encoding::Double, Encoding::Double],
    );
}

/// Start a one-shot location request: create the manager, register the handler
/// as its delegate, and ask for permission. The fix (or denial) arrives later
/// in the handler's delegate methods.
pub fn request(handler: &Handler) {
    let Some(cls) = AnyClass::get(c"CLLocationManager") else {
        eprintln!("location: CoreLocation unavailable (dev mode without bundle?)");
        return;
    };

    // SAFETY: [[CLLocationManager alloc] init] returns an owned (+1) manager.
    let manager: Retained<NSObject> = unsafe {
        let alloced: *mut AnyObject = msg_send![cls, alloc];
        let inited: *mut AnyObject = msg_send![alloced, init];
        let Some(m) = Retained::from_raw(inited.cast::<NSObject>()) else {
            return;
        };
        m
    };

    // SAFETY: setDelegate:/setDesiredAccuracy: are void sends; ~3 km suits sunrise/sunset.
    unsafe {
        let _: () = msg_send![&*manager, setDelegate: handler];
        let _: () = msg_send![&*manager, setDesiredAccuracy: 3000.0_f64];
    }

    // Keep the manager alive across the async request.
    *handler.ivars().location_manager.borrow_mut() = Some(manager.clone());

    // SAFETY: -authorizationStatus (i32) / -requestWhenInUseAuthorization (void). When already
    // authorized, the delegate's didChangeAuthorization fires and kicks off requestLocation.
    unsafe {
        let status: i32 = msg_send![&*manager, authorizationStatus];
        match status {
            DENIED | RESTRICTED => {
                *handler.ivars().location_manager.borrow_mut() = None;
                denied_alert();
            }
            _ => {
                let _: () = msg_send![&*manager, requestWhenInUseAuthorization];
            }
        }
    }
}

/// `locationManagerDidChangeAuthorization:` — once the user grants access, ask
/// for a single location fix; if they deny it, drop the manager and tell them.
pub fn handle_auth_change(handler: &Handler, manager: &AnyObject) {
    // SAFETY: -authorizationStatus returns a CLAuthorizationStatus (i32); -requestLocation is void.
    unsafe {
        let status: i32 = msg_send![manager, authorizationStatus];
        match status {
            AUTHORIZED_ALWAYS | AUTHORIZED_WHEN_IN_USE => {
                let _: () = msg_send![manager, requestLocation];
            }
            DENIED | RESTRICTED => {
                *handler.ivars().location_manager.borrow_mut() = None;
                denied_alert();
            }
            _ => {} // still NOT_DETERMINED: the prompt is on screen, wait for the answer
        }
    }
}

/// `locationManager:didUpdateLocations:` — read the latest fix, persist it as the
/// precise auto-dim location, restart the scheduler and refresh the UI.
pub fn handle_update(handler: &Handler, locations: &AnyObject) {
    // SAFETY: locations is NSArray<CLLocation>*; -coordinate returns the struct by value.
    let coord = unsafe {
        let count: usize = msg_send![locations, count];
        if count == 0 {
            return;
        }
        let loc: *mut AnyObject = msg_send![locations, lastObject];
        if loc.is_null() {
            return;
        }
        let c: Coordinate = msg_send![loc, coordinate];
        c
    };

    // One-shot done: release the manager.
    *handler.ivars().location_manager.borrow_mut() = None;

    if !coord.latitude.is_finite() || !coord.longitude.is_finite() {
        return;
    }
    if !(-90.0..=90.0).contains(&coord.latitude) || !(-180.0..=180.0).contains(&coord.longitude) {
        return;
    }
    preferences::save_location(coord.latitude, coord.longitude);
    schedule::restart();
    handler.sync_autodim_menu();
    settings::refresh_location(handler); // fill the fields if the window is open
}

/// `locationManager:didFailWithError:` — log and release the manager.
pub fn handle_fail(handler: &Handler, _error: &AnyObject) {
    eprintln!("location: request failed");
    *handler.ivars().location_manager.borrow_mut() = None;
}

/// Tell the user location access is off and where to turn it back on.
fn denied_alert() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSAlert;
    use objc2_foundation::NSString;

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(crate::i18n::s(
        "Location access is off",
        "Accès à la localisation désactivé",
    )));
    alert.setInformativeText(&NSString::from_str(crate::i18n::s(
        "Enable it in System Settings ▸ Privacy & Security ▸ Location Services, \
         or enter your latitude/longitude manually.",
        "Activez-le dans Réglages Système ▸ Confidentialité et sécurité ▸ Service de localisation, \
         ou saisissez votre latitude/longitude manuellement.",
    )));
    alert.runModal();
}
