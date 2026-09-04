use std::ffi::c_void;

use crate::abi::{ak_bytes, ak_callback, ak_event, ak_event_kind, ak_host_debt};
use crate::guard_void;

#[derive(Clone, Copy)]
pub(crate) struct HostPtr(pub(crate) *mut c_void);

unsafe impl Send for HostPtr {}
unsafe impl Sync for HostPtr {}

impl HostPtr {
    pub(crate) fn null() -> Self {
        Self(std::ptr::null_mut())
    }
}

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

    fn emit(&self, call_ctx: HostPtr, event: &ak_event) {
        guard_void(|| unsafe { (self.callback)(self.runtime_ctx.0, call_ctx.0, event) });
    }

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

    pub(crate) fn signal(&self, call_ctx: HostPtr, kind: ak_event_kind) {
        self.deliver(call_ctx, kind, ak_bytes::none(), 0);
    }

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
