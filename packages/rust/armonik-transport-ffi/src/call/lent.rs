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

/// What follows every lent buffer, checked when the buffer comes back: a host that wrote past
/// its end wrote over this first.
const SENTINEL: [u8; 8] = [0xde, 0xad, 0xbe, 0xef, 0xa5, 0x5a, 0xc3, 0x3c];

/// A buffer the host is filling.
///
/// It owns its call, as `Payload` does. The bytes are charged to the runtime's ledger until the
/// buffer comes back, and only the call can take that charge off; a weak reference that failed to
/// upgrade would be a charge no one is left to release.
#[repr(C)]
pub(crate) struct Lent {
    pub(super) tag: u64,
    pub(super) call: Arc<CallState>,
    /// The arena: `prefix` bytes kept for the gRPC prefix of a call's one request, the `len`
    /// bytes lent to the host, then the sentinel. Its length is the prefix's until the host says
    /// how many bytes it wrote, since the others are not this library's to read before then.
    pub(super) data: Vec<u8>,
    pub(super) prefix: usize,
    pub(super) len: usize,
}

/// The host wrote past the buffer it was lent, or says it wrote more than that.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Overrun;

impl Lent {
    pub(crate) fn call(&self) -> &Arc<CallState> {
        &self.call
    }

    /// The message is the `written` bytes the host wrote from the start of the buffer.
    pub(crate) fn seal(&mut self, written: usize) -> Result<(), Overrun> {
        seal(&mut self.data, self.prefix, self.len, written)
    }

    pub(crate) fn intact(&self) -> bool {
        intact(&self.data, self.prefix, self.len)
    }

    /// The call and the bytes it was charged, the arena forgotten rather than freed: past an
    /// overrun, what was overwritten may be the allocator's own record of it.
    #[allow(clippy::boxed_local)]
    pub(crate) fn abandon(self: Box<Self>) -> (Arc<CallState>, usize) {
        let Lent {
            call, data, len, ..
        } = *self;
        std::mem::forget(data);
        (call, len)
    }
}

// The `Vec` and not a slice of it: the sentinel is in its spare capacity, past what a slice's
// pointer may read.
#[allow(clippy::ptr_arg)]
fn intact(data: &Vec<u8>, prefix: usize, len: usize) -> bool {
    // SAFETY: `arena` wrote the sentinel there, within the capacity it reserved, `Vec::as_ptr`
    // reads the whole allocation, and this library writes nothing of its own past `prefix + len`.
    let sentinel =
        unsafe { std::slice::from_raw_parts(Vec::as_ptr(data).add(prefix + len), SENTINEL.len()) };
    sentinel == SENTINEL
}

fn seal(data: &mut Vec<u8>, prefix: usize, len: usize, written: usize) -> Result<(), Overrun> {
    if written > len || !intact(data, prefix, len) {
        return Err(Overrun);
    }
    // SAFETY: the prefix is initialized here, the next `written` bytes by the host, which says it
    // wrote them, and all of them are within the capacity `arena` reserved.
    unsafe { data.set_len(prefix + written) };
    Ok(())
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

/// Room for `len` bytes after `prefix` zeroed ones, with the sentinel after them. The `len` bytes
/// are left as the allocator gives them: only what the host says it wrote is ever read.
pub(super) fn arena(prefix: usize, len: usize) -> Result<Vec<u8>, ak_status> {
    let total = prefix
        .checked_add(len)
        .and_then(|bytes| bytes.checked_add(SENTINEL.len()))
        .ok_or(ak_status::AK_STATUS_INTERNAL)?;
    let mut data = Vec::new();
    data.try_reserve_exact(total)
        .map_err(|_| ak_status::AK_STATUS_INTERNAL)?;
    data.resize(prefix, 0);
    // SAFETY: `prefix + len + SENTINEL.len()` bytes were reserved.
    unsafe {
        data.as_mut_ptr()
            .add(prefix + len)
            .copy_from_nonoverlapping(SENTINEL.as_ptr(), SENTINEL.len());
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A message is what the host says it wrote, up to what it was lent.
    #[test]
    fn a_message_is_the_bytes_the_host_says_it_wrote() {
        let mut data = arena(5, 16).expect("an arena");
        assert_eq!(
            data, [0; 5],
            "only the prefix is read before the host writes"
        );
        // SAFETY: within the 16 bytes lent after the prefix.
        unsafe {
            data.as_mut_ptr()
                .add(5)
                .copy_from_nonoverlapping(b"hello".as_ptr(), 5)
        };

        assert_eq!(seal(&mut data, 5, 16, 5), Ok(()));
        assert_eq!(&data[..], b"\0\0\0\0\0hello");
    }

    /// Saying more than was lent, or writing past it, is an overrun.
    #[test]
    fn an_overrun_is_refused_whether_it_is_said_or_written() {
        let mut said = arena(0, 16).expect("an arena");
        assert_eq!(seal(&mut said, 0, 16, 17), Err(Overrun));
        assert!(said.is_empty());

        let mut written = arena(0, 16).expect("an arena");
        // SAFETY: one byte past what was lent, onto the sentinel, within the capacity.
        unsafe { written.as_mut_ptr().add(16).write(0) };
        assert!(!intact(&written, 0, 16));
        assert_eq!(seal(&mut written, 0, 16, 16), Err(Overrun));

        assert!(intact(
            &arena(5, 1024 * 1024).expect("an arena"),
            5,
            1024 * 1024
        ));
    }
}
