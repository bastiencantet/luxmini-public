//! "Launch at Login" toggle.
//!
//! macOS 13+: uses `SMAppService.mainAppService`, the modern one-liner API.
//! macOS 11-12: writes a `LaunchAgent` plist to ~/Library/LaunchAgents so the
//! user's launchd runs the app at login. Both paths are exposed through this
//! module's public API (`is_enabled` / `set_enabled`).

use std::ffi::{c_char, c_int, CStr};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Once;

use objc2::runtime::{AnyClass, AnyObject};

const BUNDLE_ID: &str = "com.bastiencantet.luxmini";
const RTLD_NOW: c_int = 2;

/// Whether the app is currently registered to launch at login.
#[must_use]
pub fn is_enabled() -> bool {
    if sm_app_service_class().is_some() {
        sm_status_is_enabled()
    } else {
        launch_agent_plist_path().exists()
    }
}

/// Enable or disable launch-at-login; errors if the OS API rejects the change.
pub fn set_enabled(enabled: bool) -> io::Result<()> {
    if sm_app_service_class().is_some() {
        sm_set_registered(enabled)
    } else {
        launch_agent_set(enabled)
    }
}

// ────────────────────────── SMAppService (macOS 13+) ──────────────────────────

fn sm_app_service_class() -> Option<&'static AnyClass> {
    // ServiceManagement.framework is present on macOS 11+, but the
    // SMAppService class itself only exists on macOS 13+. Load the framework
    // lazily so older systems don't fail to launch, then probe for the class.
    static LOAD_ONCE: Once = Once::new();
    LOAD_ONCE.call_once(|| {
        const FRAMEWORK_PATH: &CStr =
            c"/System/Library/Frameworks/ServiceManagement.framework/ServiceManagement";
        // SAFETY: loading a system framework by absolute 'static c-string path; the handle is
        // intentionally ignored (we only need the framework's classes registered with the runtime).
        unsafe {
            let _ = dlopen(FRAMEWORK_PATH.as_ptr(), RTLD_NOW);
        }
    });
    AnyClass::get(c"SMAppService")
}

fn sm_main_app_service() -> Option<*mut AnyObject> {
    let cls = sm_app_service_class()?;
    // SAFETY: cls is SMAppService; mainAppService is a class method returning an autoreleased
    // instance or nil, which we null-check before handing back.
    unsafe {
        let svc: *mut AnyObject = objc2::msg_send![cls, mainAppService];
        if svc.is_null() {
            None
        } else {
            Some(svc)
        }
    }
}

fn sm_status_is_enabled() -> bool {
    let Some(svc) = sm_main_app_service() else {
        return false;
    };
    // SAFETY: svc is the non-null SMAppService instance from sm_main_app_service(); -status
    // returns SMAppServiceStatus as an NSInteger (i64 here).
    unsafe {
        // SMAppServiceStatus: Enabled = 1 (NotRegistered=0, RequiresApproval=2, NotFound=3).
        let status: i64 = objc2::msg_send![svc, status];
        status == 1
    }
}

fn sm_set_registered(enabled: bool) -> io::Result<()> {
    let Some(svc) = sm_main_app_service() else {
        return Err(io::Error::other("SMAppService unavailable"));
    };
    // SAFETY: svc is the non-null SMAppService instance; register/unregisterAndReturnError: take
    // an NSError** out-param and return a BOOL; err is observed (not retained) by us.
    unsafe {
        let mut err: *mut AnyObject = std::ptr::null_mut();
        let ok: bool = if enabled {
            objc2::msg_send![svc, registerAndReturnError: &mut err]
        } else {
            objc2::msg_send![svc, unregisterAndReturnError: &mut err]
        };
        if ok {
            Ok(())
        } else {
            Err(io::Error::other(
                "SMAppService register/unregister returned false",
            ))
        }
    }
}

// ────────────────────────── LaunchAgent fallback (macOS 11-12) ────────────────

fn launch_agent_plist_path() -> PathBuf {
    // The /tmp fallback is a deliberate no-op sentinel for HOME-less environments
    // (launchd won't read a LaunchAgent from there).
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home)
        .join("Library/LaunchAgents")
        .join(format!("{BUNDLE_ID}.plist"))
}

fn launch_agent_set(enabled: bool) -> io::Result<()> {
    let path = launch_agent_plist_path();
    if enabled {
        let exe = std::env::current_exe()?;
        let exe_str = exe
            .to_str()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "non-utf8 exe path"))?;
        // XML-escape & in the path. Other unsafe chars (<, >, ") aren't
        // allowed in a real bundle path so we skip them.
        let safe_exe = exe_str.replace('&', "&amp;");
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{BUNDLE_ID}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{safe_exe}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <false/>
</dict>
</plist>
"#
        );
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, plist)?;
    } else if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

// Minimal libc bindings so we don't pull in the libc crate for one symbol.
extern "C" {
    fn dlopen(path: *const c_char, mode: std::ffi::c_int) -> *mut std::ffi::c_void;
}
