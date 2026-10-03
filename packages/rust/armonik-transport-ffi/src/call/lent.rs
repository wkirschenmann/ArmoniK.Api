use std::ffi::c_void;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use bytes::Bytes;

use super::CallState;
use crate::abi::{ak_bytes, ak_status};
use crate::ledger::Received;
use crate::tagged::take_tagged;

// Written into the boxes the host is given a pointer to, and checked before either is read back:
// what comes back over the ABI is whatever the host passed, and the tag is all that tells a buffer
// from a payload, or either from a pointer this library never handed out.
pub(super) const LENT_TAG: u64 = 0x414b_5f4c_454e_5400;
const PAYLOAD_TAG: u64 = 0x414b_5f50_4159_4c00;

/// A buffer the host is filling.
///
/// It owns its call, as `Payload` does. The bytes are charged to the runtime's ledger until the
/// buffer comes back, and only the call can take that charge off; a weak reference that failed to
/// upgrade would be a charge no one is left to release.
#[repr(C)]
pub(crate) struct Lent {
    pub(super) tag: u64,
    pub(super) call: Arc<CallState>,
    pub(super) data: Vec<u8>,
}

impl Lent {
    pub(crate) fn call(&self) -> &Arc<CallState> {
        &self.call
    }
}

pub(crate) fn keep(lent: Box<Lent>, status: ak_status) -> ak_status {
    let _ = Box::into_raw(lent);
    status
}

pub(crate) unsafe fn take_lent(owner: *mut c_void) -> Option<Box<Lent>> {
    unsafe { take_tagged(owner, LENT_TAG) }
}

#[repr(C)]
pub(crate) struct Payload {
    tag: u64,
    data: Bytes,
    call: Arc<CallState>,
    returns_credit: bool,
    /// A message's bytes against the ceiling, given back with the payload. The metadata and the
    /// terminal carry none.
    _charge: Option<Received>,
}

impl Drop for Payload {
    fn drop(&mut self) {
        self.call.payload_returned(self.returns_credit);
    }
}

pub(crate) unsafe fn take_payload(owner: *mut c_void) -> Option<Box<Payload>> {
    unsafe { take_tagged(owner, PAYLOAD_TAG) }
}

pub(super) fn arena(len: usize) -> Result<Vec<u8>, ak_status> {
    let mut data = Vec::new();
    data.try_reserve_exact(len)
        .map_err(|_| ak_status::AK_STATUS_INTERNAL)?;
    data.resize(len, 0);
    Ok(data)
}

pub(super) fn lend_payload(
    call: &Arc<CallState>,
    data: Bytes,
    returns_credit: bool,
    charge: Option<Received>,
) -> ak_bytes {
    call.debt.payloads.fetch_add(1, Ordering::AcqRel);
    call.ledger.hold();

    let payload = Box::new(Payload {
        tag: PAYLOAD_TAG,
        data,
        call: Arc::clone(call),
        returns_credit,
        _charge: charge,
    });
    let ptr = payload.data.as_ptr();
    let len = payload.data.len();
    ak_bytes {
        ptr,
        len,
        owner: Box::into_raw(payload) as *mut c_void,
    }
}
