//! The host's side of the boundary: the callback, and the pointers only it understands.

use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::abi::{ak_bytes, ak_callback, ak_event, ak_event_kind, ak_host_debt};

#[derive(Clone, Copy)]
/// A pointer the host owns and this library only ever hands back.
///
/// `Send` and `Sync` because the ABI says so: the host keeps `runtime_ctx` and `call_ctx` valid
/// across threads until the events that end them, and this library never dereferences either.
pub(crate) struct HostPtr(pub(crate) *mut c_void);

// SAFETY: an opaque token this library never reads. Keeping it valid is the host's obligation,
// stated in the header against the events that end it.
unsafe impl Send for HostPtr {}
// SAFETY: as above.
unsafe impl Sync for HostPtr {}

impl HostPtr {
    pub(crate) fn null() -> Self {
        Self(std::ptr::null_mut())
    }
}

/// Where a runtime's events go.
pub(crate) struct Host {
    callback: ak_callback,
    runtime_ctx: HostPtr,
}

impl Host {
    pub(crate) fn new(callback: ak_callback, runtime_ctx: *mut c_void) -> Self {
        Self {
            callback,
            runtime_ctx: HostPtr(runtime_ctx),
        }
    }

    /// Invokes the callback, and swallows a panic raised on this side of it: unwinding into C
    /// is undefined, so it stops here.
    fn emit(&self, call_ctx: HostPtr, event: &ak_event) {
        let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
            (self.callback)(self.runtime_ctx.0, call_ctx.0, event)
        }));
    }

    /// An event carrying a payload the host must consume. `status_code` is the terminal's, and
    /// zero on every other kind.
    pub(crate) fn deliver(
        &self,
        call_ctx: HostPtr,
        kind: ak_event_kind,
        payload: ak_bytes,
        status_code: i32,
    ) {
        self.emit(
            call_ctx,
            &ak_event {
                kind,
                payload,
                status_code,
                host_debt: ak_host_debt::AK_HOST_NOTHING_TO_RETURN,
            },
        );
    }

    /// An event with nothing to give back.
    pub(crate) fn signal(&self, call_ctx: HostPtr, kind: ak_event_kind) {
        self.emit(
            call_ctx,
            &ak_event {
                kind,
                payload: ak_bytes::none(),
                status_code: 0,
                host_debt: ak_host_debt::AK_HOST_NOTHING_TO_RETURN,
            },
        );
    }

    /// The runtime has stopped, and says whether the host still holds anything of it.
    pub(crate) fn signal_shutdown(&self, debt: ak_host_debt) {
        self.emit(
            HostPtr::null(),
            &ak_event {
                kind: ak_event_kind::AK_EVENT_SHUTDOWN_COMPLETE,
                payload: ak_bytes::none(),
                status_code: 0,
                host_debt: debt,
            },
        );
    }
}
