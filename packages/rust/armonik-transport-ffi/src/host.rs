use std::ffi::c_void;

use crate::abi::{ak_bytes, ak_call_ctx, ak_event, ak_event_kind, ak_host_debt};

#[derive(Clone, Copy)]
pub(crate) struct HostPtr(pub(crate) *mut c_void);

unsafe impl Send for HostPtr {}
unsafe impl Sync for HostPtr {}

impl HostPtr {
    pub(crate) fn null() -> Self {
        Self(std::ptr::null_mut())
    }
}

/// What `ak_callback` points at once `ak_runtime_create` has refused a null one.
pub(crate) type Callback = unsafe extern "C" fn(
    runtime_ctx: *mut c_void,
    call_ctx: ak_call_ctx,
    events: *const ak_event,
    count: usize,
);

pub(crate) struct Host {
    callback: Callback,
    runtime_ctx: HostPtr,
}

impl Host {
    pub(crate) fn new(callback: Callback, runtime_ctx: *mut c_void) -> Self {
        Self {
            callback,
            runtime_ctx: HostPtr(runtime_ctx),
        }
    }

    fn emit(&self, call_ctx: HostPtr, events: &[ak_event]) {
        crate::held::assert_none_held();
        debug_assert!(!events.is_empty(), "a callback carries at least one event");
        unsafe {
            (self.callback)(
                self.runtime_ctx.0,
                call_ctx.0,
                events.as_ptr(),
                events.len(),
            )
        }
    }

    /// One call's events, in delivery order, in one callback.
    pub(crate) fn deliver(&self, call_ctx: HostPtr, events: &[ak_event]) {
        self.emit(call_ctx, events);
    }

    pub(crate) fn signal(&self, call_ctx: HostPtr, kind: ak_event_kind) {
        self.emit(
            call_ctx,
            &[ak_event {
                kind,
                payload: ak_bytes::none(),
                status_code: 0,
                host_debt: ak_host_debt::AK_HOST_NOTHING_TO_RETURN,
            }],
        );
    }

    pub(crate) fn signal_runtime(&self, kind: ak_event_kind, debt: ak_host_debt) {
        self.emit(
            HostPtr::null(),
            &[ak_event {
                kind,
                payload: ak_bytes::none(),
                status_code: 0,
                host_debt: debt,
            }],
        );
    }
}
