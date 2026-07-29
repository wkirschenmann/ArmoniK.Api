//! Keeping panics on the Rust side of the ABI.
//!
//! A panic unwinding into the calling .NET frame is undefined behaviour. Every `extern "C"` entry
//! point in this crate runs its body through one of the functions below instead of running
//! directly, so a panic turns into the crate's own error status rather than crossing the boundary.

use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::error::ak_bytes;

/// Run a fallible entry point that reports failures through an `out_err: *mut ak_bytes` parameter.
///
/// On a panic, the payload is rendered into `out_err` (when non-null) exactly like any other
/// error, and [`crate::status::INTERNAL_PANIC`] is returned.
pub(crate) fn catch_unwind_status(out_err: *mut ak_bytes, body: impl FnOnce() -> i32) -> i32 {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(status) => status,
        Err(payload) => {
            if !out_err.is_null() {
                // SAFETY: every caller of this function documents `out_err` as writable.
                unsafe { *out_err = ak_bytes::from_bytes(panic_message(payload.as_ref())) };
            }
            crate::status::INTERNAL_PANIC
        }
    }
}

/// Run a fallible entry point that has no way to report an error message, only a status code.
pub(crate) fn catch_unwind_status_only(body: impl FnOnce() -> i32) -> i32 {
    catch_unwind(AssertUnwindSafe(body)).unwrap_or(crate::status::INTERNAL_PANIC)
}

/// Run an infallible entry point (one that returns nothing, such as a `_free` function).
///
/// A panic is swallowed: there is no return value to signal it through, and these functions must
/// not be allowed to abort the process either.
pub(crate) fn catch_unwind_void(body: impl FnOnce()) {
    let _ = catch_unwind(AssertUnwindSafe(body));
}

/// Run an entry point whose return type is not a status code, falling back to `fallback` on a panic.
///
/// Used by the accessors that answer with a value rather than a status — the fallback has to be a
/// value the caller already has to handle, such as a null pointer.
pub(crate) fn catch_unwind_or<T>(fallback: T, body: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(body)).unwrap_or(fallback)
}

/// Render a panic payload as a human-readable message.
pub(crate) fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        format!("panicked: {message}")
    } else if let Some(message) = payload.downcast_ref::<String>() {
        format!("panicked: {message}")
    } else {
        String::from("panicked with a non-string payload")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_becomes_the_panic_status_and_is_rendered() {
        let mut out_err = ak_bytes::EMPTY;
        let result = catch_unwind_status(std::ptr::addr_of_mut!(out_err), || {
            panic!("boom");
        });

        assert_eq!(result, crate::status::INTERNAL_PANIC);
        assert!(
            !out_err.ptr.is_null(),
            "the panic message should be written"
        );
        // SAFETY: just written above by this same test, and freed right after.
        let message = unsafe { std::slice::from_raw_parts(out_err.ptr, out_err.len) };
        assert!(String::from_utf8_lossy(message).contains("boom"));
        unsafe { crate::error::ak_bytes_free(out_err) };
    }

    #[test]
    fn a_panic_is_reported_even_without_an_error_slot() {
        let result = catch_unwind_status(std::ptr::null_mut(), || {
            panic!("boom");
        });
        assert_eq!(result, crate::status::INTERNAL_PANIC);
    }

    #[test]
    fn a_normal_return_passes_through() {
        let mut out_err = ak_bytes::EMPTY;
        let result = catch_unwind_status(std::ptr::addr_of_mut!(out_err), || 42);
        assert_eq!(result, 42);
        assert!(
            out_err.ptr.is_null(),
            "no error should be written on success"
        );
    }

    #[test]
    fn status_only_reports_the_panic_status_without_a_message() {
        assert_eq!(
            catch_unwind_status_only(|| panic!("boom")),
            crate::status::INTERNAL_PANIC
        );
        assert_eq!(catch_unwind_status_only(|| 7), 7);
    }

    #[test]
    fn void_swallows_the_panic() {
        catch_unwind_void(|| panic!("boom"));
        // Reaching here at all is the assertion.
    }

    #[test]
    fn panic_message_reads_string_payloads() {
        let result = catch_unwind(AssertUnwindSafe(|| -> () {
            panic!("specific message");
        }));
        let Err(payload) = result else {
            panic!("expected a panic");
        };
        assert!(panic_message(payload.as_ref()).contains("specific message"));
    }
}
