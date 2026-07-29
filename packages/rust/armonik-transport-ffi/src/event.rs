//! The wake-up primitive the caller waits on.
//!
//! # Why Rust creates it
//!
//! The obvious design has .NET create the event and pass it in. It does not work cleanly: .NET
//! needs the handle to stay valid for as long as it is waiting on the call, and Rust needs it to
//! stay valid for as long as the driving task might still signal. Those two windows *overlap*
//! rather than nest, so neither side can be the sole owner, and both trying to close it is a
//! double close.
//!
//! Inverting it removes the problem. Rust creates the event, owns it, and hands out a borrowed
//! handle through [`crate::ak_call_wait_handle`]. The handle lives in an [`Arc`], cloned into the
//! driving task, so whichever of the task and [`crate::ak_call_free`] finishes last closes it —
//! entirely within Rust, with no cross-language coordination. On the .NET side the borrow is
//! expressed in the type system: `new SafeWaitHandle(handle, ownsHandle: false)`.
//!
//! The borrow is valid exactly as long as the `ak_call` it came from, which is the same rule that
//! already governs the call handle itself, so it adds no new lifetime for the caller to track.

use std::ffi::c_void;
use std::sync::Arc;

/// An auto-reset event owned by this crate.
///
/// Auto-reset rather than manual-reset: each `SetEvent` releases exactly one waiter and the event
/// returns to unsignalled by itself, which is the semantics a "poll again, there is new state"
/// notification wants. A manual-reset event would stay signalled and spin the .NET waiter until it
/// reset it explicitly.
#[derive(Debug)]
pub(crate) struct OwnedEvent {
    handle: *mut c_void,
}

// SAFETY: a Win32 HANDLE is an opaque value with no thread affinity. `SetEvent` is documented as
// safe to call on it from any thread, any number of times, and `CloseHandle` runs exactly once,
// from `Drop`, when the last `Arc` goes away.
unsafe impl Send for OwnedEvent {}
// SAFETY: as above.
unsafe impl Sync for OwnedEvent {}

impl OwnedEvent {
    /// Create a fresh, unsignalled auto-reset event.
    pub(crate) fn new() -> std::io::Result<Arc<Self>> {
        let handle = create_event()?;
        Ok(Arc::new(Self { handle }))
    }

    /// The raw handle, for the caller to wait on. Borrowed: never closed by the caller.
    pub(crate) fn raw(&self) -> *mut c_void {
        self.handle
    }

    /// Wake one waiter.
    pub(crate) fn signal(&self) {
        if self.handle.is_null() {
            return;
        }
        set_event(self.handle);
    }
}

impl Drop for OwnedEvent {
    fn drop(&mut self) {
        if self.handle.is_null() {
            return;
        }
        close_handle(self.handle);
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::c_void;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateEventW(
            attributes: *mut c_void,
            manual_reset: i32,
            initial_state: i32,
            name: *const u16,
        ) -> *mut c_void;
        fn SetEvent(event: *mut c_void) -> i32;
        fn CloseHandle(object: *mut c_void) -> i32;
    }

    pub(super) fn create_event() -> std::io::Result<*mut c_void> {
        // SAFETY: null attributes and name are the documented way to ask for a default, unnamed
        // event; the two flags are plain booleans.
        let handle = unsafe {
            CreateEventW(
                std::ptr::null_mut(),
                0, // auto-reset
                0, // initially unsignalled
                std::ptr::null(),
            )
        };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        Ok(handle)
    }

    pub(super) fn set_event(handle: *mut c_void) {
        // SAFETY: `handle` came from `create_event` and is not closed until `OwnedEvent::drop`,
        // which cannot run while a caller still holds the `Arc` this is reached through.
        unsafe {
            SetEvent(handle);
        }
    }

    pub(super) fn close_handle(handle: *mut c_void) {
        // SAFETY: called exactly once, from `OwnedEvent::drop`, on a handle from `create_event`.
        unsafe {
            CloseHandle(handle);
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use std::ffi::c_void;

    /// This crate's cdylib only ever ships for Windows. These stubs exist so `cargo
    /// test`/`clippy`/`fmt`/`doc` succeed on the Linux CI legs, where the tests poll instead of
    /// waiting on a handle.
    pub(super) fn create_event() -> std::io::Result<*mut c_void> {
        Ok(std::ptr::null_mut())
    }

    pub(super) fn set_event(_handle: *mut c_void) {}

    pub(super) fn close_handle(_handle: *mut c_void) {}
}

use platform::{close_handle, create_event, set_event};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_event_can_be_created_and_signalled() {
        let event = OwnedEvent::new().expect("create");
        // Signalling is idempotent from the caller's point of view: an auto-reset event that nobody
        // is waiting on just stays signalled until someone waits.
        event.signal();
        event.signal();
    }

    #[test]
    fn the_handle_survives_every_clone_and_is_closed_once() {
        let event = OwnedEvent::new().expect("create");
        let raw = event.raw();

        let clone = Arc::clone(&event);
        drop(event);
        // The first `Arc` is gone but the clone keeps the handle alive, which is exactly what lets
        // the driving task outlive `ak_call_free` without ever touching a closed handle.
        assert_eq!(clone.raw(), raw);
        clone.signal();
    }

    #[cfg(windows)]
    #[test]
    fn a_signalled_event_releases_a_waiter() {
        use std::ffi::c_void;

        #[link(name = "kernel32")]
        extern "system" {
            fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
        }

        const WAIT_OBJECT_0: u32 = 0;
        const WAIT_TIMEOUT: u32 = 0x0000_0102;

        let event = OwnedEvent::new().expect("create");

        // Nothing has signalled it yet, so a zero-timeout wait must report a timeout. Without this
        // half, the assertion below would also pass on an event that was somehow born signalled.
        // SAFETY: a live handle from `OwnedEvent::new`, held by `event` across the call.
        let before = unsafe { WaitForSingleObject(event.raw(), 0) };
        assert_eq!(before, WAIT_TIMEOUT, "a fresh event must be unsignalled");

        event.signal();

        // SAFETY: as above.
        let after = unsafe { WaitForSingleObject(event.raw(), 0) };
        assert_eq!(after, WAIT_OBJECT_0, "signalling must release a waiter");

        // Auto-reset: consuming the signal above must have reset it.
        // SAFETY: as above.
        let again = unsafe { WaitForSingleObject(event.raw(), 0) };
        assert_eq!(again, WAIT_TIMEOUT, "an auto-reset event must reset itself");
    }
}
