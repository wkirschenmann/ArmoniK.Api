//! The host's side of the boundary: the callback, and the pointers only it understands.

use std::ffi::c_void;

use crate::abi::{ak_bytes, ak_callback, ak_event, ak_event_kind, ak_host_debt};
use crate::guard_void;

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
    /// is undefined, so it stops here - the same rule the entry points answer to, and the one
    /// place it is an upcall rather than a return.
    fn emit(&self, call_ctx: HostPtr, event: &ak_event) {
        guard_void(|| unsafe { (self.callback)(self.runtime_ctx.0, call_ctx.0, event) });
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

    /// An event of one call with nothing to give back.
    pub(crate) fn signal(&self, call_ctx: HostPtr, kind: ak_event_kind) {
        self.deliver(call_ctx, kind, ak_bytes::none(), 0);
    }

    /// An event of the runtime itself, which belongs to no call and so carries no call context.
    ///
    /// `debt` is what the host still holds; it is meaningful on the shutdown and zero after.
    pub(crate) fn signal_runtime(&self, kind: ak_event_kind, debt: ak_host_debt) {
        self.emit(
            HostPtr::null(),
            &ak_event {
                kind,
                payload: ak_bytes::none(),
                status_code: 0,
                host_debt: debt,
            },
        );
    }
}
