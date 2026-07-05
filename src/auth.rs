//! Authorization Services helper for the "install privileged tool" prompt: loads
//! Security.framework's (deprecated but shipping) C API via dlopen/dlsym so the
//! prompt is branded with the app. Returns None if the API is gone (caller falls
//! back to osascript).

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::path::Path;

extern "C" {
    fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn fread(buf: *mut c_void, size: usize, nmemb: usize, stream: *mut c_void) -> usize;
    fn fclose(stream: *mut c_void) -> c_int;
}

const RTLD_NOW: c_int = 2;
const ERR_SUCCESS: i32 = 0;
const ERR_CANCELED: i32 = -60006;

const SECURITY_PATH: &CStr = c"/System/Library/Frameworks/Security.framework/Security";

/// `AuthorizationCreate` from Security.framework.
type AuthorizationCreateFn = unsafe extern "C" fn(
    rights: *const c_void,
    environment: *const c_void,
    flags: u32,
    authorization: *mut *mut c_void,
) -> i32;

/// `AuthorizationExecuteWithPrivileges` from Security.framework.
type AuthorizationExecuteFn = unsafe extern "C" fn(
    authorization: *mut c_void,
    path_to_tool: *const c_char,
    options: u32,
    arguments: *const *const c_char,
    communications_pipe: *mut *mut c_void, // FILE**
) -> i32;

/// `AuthorizationFree` from Security.framework.
type AuthorizationFreeFn = unsafe extern "C" fn(authorization: *mut c_void, flags: u32) -> i32;

/// Run `chown root && chmod u+s` on `path` via a privileged shell spawned
/// through Authorization Services. The OS shows a standard admin prompt
/// branded with the calling app (icon + name).
///
/// Returns:
///   None             — API not available, caller should fall back
///   Some(Ok(()))     — helper is now setuid root
///   Some(Err(...))   — API available but elevation failed (cancel / bad path / …)
pub fn try_elevate_setuid(path: &Path) -> Option<std::io::Result<()>> {
    let Some(p) = path.to_str() else {
        return Some(Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "bad path",
        )));
    };
    // Same safety guard as the osascript path: refuse paths containing quotes
    // so the shell command below can't be broken out of.
    if p.contains('\'') || p.contains('"') {
        return Some(Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "path contains quote",
        )));
    }

    // SAFETY: symbols loaded from the system Security.framework by absolute path, each
    // null-checked; transmuted fn pointers mirror the C signatures; all resources freed once.
    unsafe {
        let handle = dlopen(SECURITY_PATH.as_ptr(), RTLD_NOW);
        if handle.is_null() {
            return None;
        }

        // SAFETY: dlsym results are null-checked before being transmuted/called.
        let create_sym = dlsym(handle, c"AuthorizationCreate".as_ptr());
        let exec_sym = dlsym(handle, c"AuthorizationExecuteWithPrivileges".as_ptr());
        let free_sym = dlsym(handle, c"AuthorizationFree".as_ptr());
        if create_sym.is_null() || exec_sym.is_null() || free_sym.is_null() {
            return None;
        }

        // SAFETY: the fn-pointer types mirror the C signatures (turbofish catches drift).
        let create = std::mem::transmute::<*mut c_void, AuthorizationCreateFn>(create_sym);
        let exec = std::mem::transmute::<*mut c_void, AuthorizationExecuteFn>(exec_sym);
        let free = std::mem::transmute::<*mut c_void, AuthorizationFreeFn>(free_sym);

        // SAFETY: null rights/env + flags 0 = default authorization; &raw mut auth is a valid out-slot.
        let mut auth: *mut c_void = std::ptr::null_mut();
        let status = create(std::ptr::null(), std::ptr::null(), 0, &raw mut auth);
        if status != ERR_SUCCESS {
            return Some(Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("AuthorizationCreate failed: {status}"),
            )));
        }

        // /bin/sh -c "chown root 'path' && chmod u+s 'path'". An interior NUL (not caught by the
        // quote-guard) fails gracefully after freeing auth.
        let cmd = format!("chown root '{p}' && chmod u+s '{p}'");
        let Ok(cmd_c) = CString::new(cmd) else {
            free(auth, 0);
            return Some(Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "path contains NUL",
            )));
        };
        let flag_c = c"-c";
        let argv: [*const c_char; 3] = [flag_c.as_ptr(), cmd_c.as_ptr(), std::ptr::null()];

        // SAFETY: auth is live; argv is NUL-terminated and its backing outlives the call.
        let mut pipe: *mut c_void = std::ptr::null_mut();
        let status = exec(auth, c"/bin/sh".as_ptr(), 0, argv.as_ptr(), &raw mut pipe);

        if status != ERR_SUCCESS {
            free(auth, 0);
            let msg = if status == ERR_CANCELED {
                "admin prompt cancelled".to_string()
            } else {
                format!("AuthorizationExecuteWithPrivileges failed: {status}")
            };
            return Some(Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                msg,
            )));
        }

        // exec returns once the tool spawns; draining stdout to EOF blocks until it exits.
        // SAFETY: pipe is exec's FILE* (null-checked); buf is valid; fclose called once.
        if !pipe.is_null() {
            let mut buf = [0u8; 256];
            loop {
                let n = fread(buf.as_mut_ptr().cast::<c_void>(), 1, buf.len(), pipe);
                if n == 0 {
                    break;
                }
            }
            fclose(pipe);
        }

        free(auth, 0);
    }

    Some(Ok(()))
}
