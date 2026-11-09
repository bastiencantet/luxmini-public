//! Setapp distribution integration.
//!
//! The Setapp framework is linked statically by the Setapp bundle script. Its
//! manager validates the app against the per-app public key in the main bundle.

use objc2::runtime::{AnyClass, AnyObject};

#[link(name = "Setapp", kind = "static")]
extern "C" {
    // Referencing the Objective-C class symbol is required for a static
    // framework. A runtime-only class lookup would let the linker discard the
    // Setapp archive as unused.
    #[link_name = "OBJC_CLASS_$_STPManager"]
    static STP_MANAGER_CLASS: AnyClass;
}

const USER_INTERACTION_EVENT: isize = 2;

fn manager() -> Option<*mut AnyObject> {
    // SAFETY: the symbol is provided by the statically linked Setapp framework
    // and STPManager exposes +sharedInstance.
    let manager: *mut AnyObject = unsafe { objc2::msg_send![&STP_MANAGER_CLASS, sharedInstance] };
    (!manager.is_null()).then_some(manager)
}

/// Initialize the framework and display Setapp's release notes when needed.
pub fn init() {
    let Some(manager) = manager() else {
        eprintln!("setapp: framework manager class not found");
        return;
    };
    // SAFETY: the Setapp manager exposes -showReleaseNotesWindowIfNeeded on macOS.
    unsafe {
        let _: () = objc2::msg_send![manager, showReleaseNotesWindowIfNeeded];
    }
    eprintln!("setapp: integration initialized");
}

/// Report opening the menu-bar UI as the interaction event Setapp requires.
pub fn report_user_interaction() {
    let Some(manager) = manager() else {
        return;
    };
    // SAFETY: STPUsageEventUserInteraction is the NSInteger enum value 2.
    unsafe {
        let _: () = objc2::msg_send![manager, reportUsageEvent: USER_INTERACTION_EVENT];
    }
}
