use std::ffi::c_void;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use bytes::Bytes;

use super::CallState;
use crate::abi::{ak_bytes, ak_status};
use crate::ledger::Received;
use crate::spares::Spares;
use crate::tagged::take_tagged;

// Written into the boxes the host is given a pointer to, and checked before either is read back:
// what comes back over the ABI is whatever the host passed, and the tag is all that tells a buffer
// from a payload, or either from a pointer this library never handed out.
pub(super) const LENT_TAG: u64 = 0x414b_5f4c_454e_5400;
const PAYLOAD_TAG: u64 = 0x414b_5f50_4159_4c00;

/// What an arena keeps ahead of the bytes lent to the host: the gRPC prefix in its last five bytes,
/// and three more, so that the host's bytes start on an eight-byte boundary where the allocation
/// does - the system allocators align a block to 16 bytes on x64 and to 8 on x86 - and only the
/// prefix is written unaligned. The message goes out from the prefix.
pub(crate) const HEADROOM: usize = 8;

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
    /// The arena: `headroom` bytes, the message's gRPC prefix in the last of them, the `len` bytes
    /// lent to the host, then the sentinel. Its length is the headroom's until the host says how
    /// many bytes it wrote, since the others are not this library's to read before then.
    pub(super) data: Vec<u8>,
    pub(super) headroom: usize,
    pub(super) len: usize,
    /// What the lend is charged: `len`, and the slack of a spare arena it took.
    pub(super) charged: usize,
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
        seal(&mut self.data, self.headroom, self.len, written)
    }

    pub(crate) fn intact(&self) -> bool {
        intact(&self.data, self.headroom, self.len)
    }

    /// Moves the lend to `data`, an arena for `len` bytes after the same headroom, charged
    /// `charged`, with the first `keep` bytes the host wrote carried over. Returns the arena it
    /// leaves.
    ///
    /// `keep` is at most what was lent and what is lent now, and the old arena is intact: both are
    /// the caller's to have checked, since an arena that was overrun is not read.
    pub(super) fn move_to(
        &mut self,
        mut data: Vec<u8>,
        len: usize,
        charged: usize,
        keep: usize,
    ) -> Vec<u8> {
        debug_assert!(keep <= self.len && keep <= len);
        // SAFETY: each arena reserved `headroom + len` bytes and more, `keep` is within both, and
        // two allocations do not overlap. The host wrote the `keep` bytes it says it did.
        unsafe {
            data.as_mut_ptr()
                .add(self.headroom)
                .copy_from_nonoverlapping(Vec::as_ptr(&self.data).add(self.headroom), keep)
        };
        self.len = len;
        self.charged = charged;
        std::mem::replace(&mut self.data, data)
    }

    /// The bytes lent to the host: where they start in the arena.
    pub(super) fn lent_ptr(&mut self) -> *mut u8 {
        // SAFETY: `arena` reserved `headroom + len` bytes and more.
        unsafe { self.data.as_mut_ptr().add(self.headroom) }
    }

    /// The call and the bytes it was charged, the arena forgotten rather than freed: past an
    /// overrun, what was overwritten may be the allocator's own record of it.
    #[allow(clippy::boxed_local)]
    pub(crate) fn abandon(self: Box<Self>) -> (Arc<CallState>, usize) {
        let Lent {
            call,
            data,
            charged,
            ..
        } = *self;
        std::mem::forget(data);
        (call, charged)
    }
}

// The `Vec` and not a slice of it: the sentinel is in its spare capacity, past what a slice's
// pointer may read.
#[allow(clippy::ptr_arg)]
fn intact(data: &Vec<u8>, headroom: usize, len: usize) -> bool {
    // SAFETY: `arena` wrote the sentinel there, within the capacity it reserved, `Vec::as_ptr`
    // reads the whole allocation, and this library writes nothing of its own past `headroom + len`.
    let sentinel = unsafe {
        std::slice::from_raw_parts(Vec::as_ptr(data).add(headroom + len), SENTINEL.len())
    };
    sentinel == SENTINEL
}

fn seal(data: &mut Vec<u8>, headroom: usize, len: usize, written: usize) -> Result<(), Overrun> {
    if written > len || !intact(data, headroom, len) {
        return Err(Overrun);
    }
    // SAFETY: the headroom is initialized here, the next `written` bytes by the host, which says it
    // wrote them, and all of them are within the capacity `arena` reserved.
    unsafe { data.set_len(headroom + written) };
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

/// What an arena of `headroom` and `len` bytes holds past them and the sentinel: a spare's slack.
pub(super) fn slack(data: &Vec<u8>, headroom: usize, len: usize) -> usize {
    data.capacity() - (headroom + len + SENTINEL.len())
}

/// Room for `len` bytes after `headroom` zeroed ones, with the sentinel after them: a spare of the
/// channel's if one fits and `spares` is given, else a new allocation. The `len` bytes are left
/// as they are, a spare's as its last message left them: only what the host says it wrote is
/// ever read.
pub(super) fn arena(
    headroom: usize,
    len: usize,
    spares: Option<&Spares>,
) -> Result<Vec<u8>, ak_status> {
    let total = headroom
        .checked_add(len)
        .and_then(|bytes| bytes.checked_add(SENTINEL.len()))
        .ok_or(ak_status::AK_STATUS_INTERNAL)?;
    let mut data = match spares.and_then(|spares| spares.take(total)) {
        Some(spare) => spare,
        None => {
            let mut data = Vec::new();
            data.try_reserve_exact(total)
                .map_err(|_| ak_status::AK_STATUS_INTERNAL)?;
            #[cfg(feature = "test-hooks")]
            crate::hooks::count_new_arena();
            data
        }
    };
    data.resize(headroom, 0);
    // SAFETY: the arena holds at least `headroom + len + SENTINEL.len()` bytes.
    unsafe {
        data.as_mut_ptr()
            .add(headroom + len)
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

    fn arena(headroom: usize, len: usize) -> Result<Vec<u8>, ak_status> {
        super::arena(headroom, len, None)
    }

    /// A message is what the host says it wrote, up to what it was lent.
    #[test]
    fn a_message_is_the_bytes_the_host_says_it_wrote() {
        let mut data = arena(5, 16).expect("an arena");
        assert_eq!(
            data, [0; 5],
            "only the headroom is read before the host writes"
        );
        // SAFETY: within the 16 bytes lent after the headroom.
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

    /// The host's bytes start on an eight-byte boundary when the allocator's blocks do, as on the
    /// target this runs on, and the headroom is a multiple of eight.
    #[test]
    fn the_bytes_lent_to_the_host_are_aligned() {
        for len in [1, 5, 4096, 64 * 1024] {
            let data = arena(HEADROOM, len).expect("an arena");
            // SAFETY: the arena holds `HEADROOM + len` bytes and more.
            let lent = unsafe { data.as_ptr().add(HEADROOM) };
            assert_eq!(lent as usize % 8, 0, "{len} bytes lent at {lent:p}");
        }
    }
}
