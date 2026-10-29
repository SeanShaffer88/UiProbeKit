//! C ABI surface — everything declared in include/uiprobekit.h.
//! Conventions (house standard):
//! - `*mut c_char` JSON results are caller-owned; free with
//!   `up_free_string`.
//! - Failures return null (or, for `up_action`, the error code)
//!   and set the thread-local last error.
//! - Nothing unwinds across the FFI boundary (catch_unwind
//!   everywhere; release profile also sets panic = "abort").
//! - Specs are parsed and validated before any platform call:
//!   a bad spec is UP_ERR_INVALID_ARG on every platform. On
//!   non-Windows platforms a valid spec reaches the platform
//!   layer and returns UP_ERR_NOT_SUPPORTED — never a fake
//!   empty result.

#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::panic::catch_unwind;

use crate::{ERR_INTERNAL, ERR_INVALID_ARG, UP_OK};

thread_local! {
    static LAST_ERROR: RefCell<(i32, CString)> =
        RefCell::new((0, CString::new("").unwrap()));
}

pub fn set_error(code: i32, msg: &str) {
    let clean: String = msg.chars().filter(|&c| c != '\0').collect();
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = (
            code,
            CString::new(clean).unwrap_or_else(|_| CString::new("error").unwrap()),
        );
    });
}

fn clear_error() {
    set_error(UP_OK, "");
}

fn out_json(result: crate::UpResult<String>) -> *mut c_char {
    match result {
        Ok(text) => match CString::new(text) {
            Ok(s) => {
                clear_error();
                s.into_raw()
            }
            Err(_) => {
                set_error(ERR_INTERNAL, "result string contained a NUL byte");
                std::ptr::null_mut()
            }
        },
        Err((code, msg)) => {
            set_error(code, &msg);
            std::ptr::null_mut()
        }
    }
}

fn cstr<'a>(ptr: *const c_char, what: &str) -> crate::UpResult<&'a str> {
    if ptr.is_null() {
        return Err((ERR_INVALID_ARG, format!("{what} must not be null")));
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map_err(|_| (ERR_INVALID_ARG, format!("{what} must be valid UTF-8")))
}

fn guard_json<F: FnOnce() -> *mut c_char>(f: F) -> *mut c_char {
    match catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(p) => p,
        Err(_) => {
            set_error(ERR_INTERNAL, "internal panic caught at the FFI boundary");
            std::ptr::null_mut()
        }
    }
}

fn guard_code<F: FnOnce() -> c_int>(f: F) -> c_int {
    match catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(c) => c,
        Err(_) => {
            set_error(ERR_INTERNAL, "internal panic caught at the FFI boundary");
            ERR_INTERNAL as c_int
        }
    }
}

#[no_mangle]
pub extern "C" fn up_version() -> *const c_char {
    static VERSION_C: &[u8] = b"0.1.0\0";
    VERSION_C.as_ptr() as *const c_char
}

#[no_mangle]
pub extern "C" fn up_last_error() -> *const c_char {
    LAST_ERROR.with(|e| e.borrow().1.as_ptr())
}

#[no_mangle]
pub extern "C" fn up_last_error_code() -> c_int {
    LAST_ERROR.with(|e| e.borrow().0) as c_int
}

/// # Safety (caller contract): `p` is null or a string this
/// library returned and has not been freed yet. Documented in
/// the header.
#[no_mangle]
pub extern "C" fn up_free_string(p: *mut c_char) {
    if !p.is_null() {
        unsafe {
            drop(CString::from_raw(p));
        }
    }
}

#[no_mangle]
pub extern "C" fn up_list_windows() -> *mut c_char {
    guard_json(|| out_json(crate::list_windows()))
}

#[no_mangle]
pub extern "C" fn up_inspect(spec_json: *const c_char) -> *mut c_char {
    guard_json(|| {
        let inner = cstr(spec_json, "spec_json").and_then(crate::inspect);
        out_json(inner)
    })
}

#[no_mangle]
pub extern "C" fn up_find(spec_json: *const c_char) -> *mut c_char {
    guard_json(|| {
        let inner = cstr(spec_json, "spec_json").and_then(crate::find);
        out_json(inner)
    })
}

#[no_mangle]
pub extern "C" fn up_action(spec_json: *const c_char) -> c_int {
    guard_code(|| {
        let inner = cstr(spec_json, "spec_json").and_then(crate::action);
        match inner {
            Ok(()) => {
                clear_error();
                UP_OK as c_int
            }
            Err((code, msg)) => {
                set_error(code, &msg);
                code as c_int
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    fn cs(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    // Used by the Windows branch of ffi_platform_contract;
    // off Windows every result is null, so nothing is taken.
    #[cfg_attr(not(windows), allow(dead_code))]
    fn take(p: *mut c_char) -> String {
        assert!(!p.is_null(), "expected a result string, got null");
        let s = unsafe { CStr::from_ptr(p) }.to_str().unwrap().to_string();
        up_free_string(p);
        s
    }

    #[test]
    fn ffi_version_and_free_null() {
        let v = unsafe { CStr::from_ptr(up_version()) }.to_str().unwrap();
        assert_eq!(v, "0.1.0");
        up_free_string(std::ptr::null_mut()); // must be a no-op
    }

    #[test]
    fn ffi_bad_specs_are_argument_errors_everywhere() {
        assert!(up_inspect(std::ptr::null()).is_null());
        assert_eq!(up_last_error_code(), ERR_INVALID_ARG as c_int);
        assert!(up_inspect(cs("{\"hwnd\": 0}").as_ptr()).is_null());
        assert_eq!(up_last_error_code(), ERR_INVALID_ARG as c_int);
        // find with no criterion
        assert!(up_find(cs("{\"hwnd\": 5}").as_ptr()).is_null());
        assert_eq!(up_last_error_code(), ERR_INVALID_ARG as c_int);
        // set_value without value
        assert_eq!(
            up_action(cs("{\"hwnd\": 5, \"path\": [], \"action\": \"set_value\"}").as_ptr()),
            ERR_INVALID_ARG as c_int
        );
        // the last-error string is populated and readable
        let msg = unsafe { CStr::from_ptr(up_last_error()) }
            .to_str()
            .unwrap()
            .to_string();
        assert!(msg.contains("value"), "unexpected message: {msg}");
    }

    #[test]
    fn ffi_platform_contract() {
        #[cfg(not(windows))]
        {
            assert!(up_list_windows().is_null());
            assert_eq!(
                up_last_error_code(),
                crate::ERR_NOT_SUPPORTED as c_int,
                "non-Windows list_windows must be a documented NOT_SUPPORTED"
            );
            assert!(up_inspect(cs("{\"hwnd\": 5}").as_ptr()).is_null());
            assert_eq!(up_last_error_code(), crate::ERR_NOT_SUPPORTED as c_int);
            assert!(up_find(cs("{\"hwnd\": 5, \"name\": \"x\"}").as_ptr()).is_null());
            assert_eq!(up_last_error_code(), crate::ERR_NOT_SUPPORTED as c_int);
            assert_eq!(
                up_action(cs("{\"hwnd\": 5, \"path\": [], \"action\": \"focus\"}").as_ptr()),
                crate::ERR_NOT_SUPPORTED as c_int
            );
        }
        #[cfg(windows)]
        {
            // On Windows the enumeration runs for real; the
            // desktop always has top-level windows. Tree and
            // runtime behaviour are a field test's to prove —
            // see QUALITY.md — so here we only assert the call
            // contract: a JSON array comes back and parses.
            let text = take(up_list_windows());
            let v = crate::json::parse(&text).unwrap();
            assert!(v.as_array().is_some());
            assert_eq!(up_last_error_code(), UP_OK as c_int);
        }
    }
}
