//! The C types the ABI is written in.
//!
//! Layouts, not behaviour: what each one means to the host is in the header. Every struct the
//! host builds carries a `struct_size` it sets to its own `sizeof`.

use std::ffi::c_void;

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// What an entry point answers.
pub enum ak_status {
    AK_STATUS_OK = 0,
    AK_STATUS_HANDLE_STALE = 1,
    AK_STATUS_SLOT_BUSY = 2,
    AK_STATUS_INVALID_ARG = 3,
    AK_STATUS_INTERNAL = 4,
    AK_STATUS_BUDGET_BUSY = 5,
    AK_STATUS_INVALID_STATE = 6,
    AK_STATUS_MESSAGE_TOO_LARGE = 7,
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// What a runtime is doing, and whether anything of it is still out.
pub enum ak_runtime_state {
    AK_RUNTIME_RUNNING = 1,
    AK_RUNTIME_GRPC_STOPPING = 2,
    AK_RUNTIME_GRPC_STOPPED = 3,
    AK_RUNTIME_QUIESCENT = 4,
    AK_RUNTIME_FAILED_UNQUIESCED = 5,
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// What a callback is announcing.
pub enum ak_event_kind {
    AK_EVENT_INITIAL_METADATA = 1,
    AK_EVENT_MESSAGE = 2,
    AK_EVENT_STATUS = 3,
    AK_EVENT_WRITE_DONE = 4,
    AK_EVENT_SHUTDOWN_COMPLETE = 5,
    AK_EVENT_RESOURCES_RELEASED = 6,
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Whether the host still holds memory of a runtime that has stopped.
///
/// Zero is "nothing outstanding", so a runtime that forgot to set the field makes the host
/// destroy too early and be refused - diagnosable, where the opposite default would make it wait
/// for an event that never comes.
pub enum ak_host_debt {
    AK_HOST_NOTHING_TO_RETURN = 0,
    AK_HOST_MUST_RETURN = 1,
}

/// A slot index and a generation, so a token from a reused slot is refused rather than aliasing
/// what took its place.
pub type ak_handle = u64;

pub const AK_HANDLE_NONE: ak_handle = 0;

#[repr(C)]
#[derive(Clone, Copy)]
/// Bytes the host lends this library for the duration of one downcall.
pub struct ak_bytes_in {
    pub ptr: *const u8,
    pub len: usize,
}

impl ak_bytes_in {
    /// What the host pointed at, or `None` if it pointed at nothing it may not.
    ///
    /// # Safety
    ///
    /// `ptr` must be valid for `len` bytes for the duration of the borrow.
    pub(crate) unsafe fn as_slice<'a>(&self) -> Option<&'a [u8]> {
        if self.len == 0 {
            return Some(&[]);
        }
        if self.ptr.is_null() {
            return None;
        }
        // SAFETY: forwarded from this function's own contract.
        Some(unsafe { std::slice::from_raw_parts(self.ptr, self.len) })
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
/// A read-only view the host owns until it calls `ak_event_consumed`.
///
/// `owner` and not `ptr` identifies the allocation: the view may be a slice of a larger buffer.
pub struct ak_bytes {
    pub ptr: *const u8,
    pub len: usize,
    pub owner: *mut c_void,
}

impl ak_bytes {
    /// The empty, unowned value that an event with no payload carries.
    pub(crate) fn none() -> Self {
        Self {
            ptr: std::ptr::null(),
            len: 0,
            owner: std::ptr::null_mut(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
/// A writable buffer lent out of a call's arena, given back exactly once.
pub struct ak_buffer {
    pub ptr: *mut u8,
    pub len: usize,
    pub owner: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
/// One event, on the callback's stack.
pub struct ak_event {
    pub kind: ak_event_kind,
    pub payload: ak_bytes,
    pub status_code: i32,
    pub host_debt: ak_host_debt,
}

/// Where every event of a runtime goes.
pub type ak_callback =
    unsafe extern "C" fn(runtime_ctx: *mut c_void, call_ctx: *mut c_void, event: *const ak_event);

#[repr(C)]
#[derive(Clone, Copy)]
/// How a runtime is built.
pub struct ak_runtime_config {
    pub struct_size: u32,
    /// Zero leaves the choice to the runtime.
    pub worker_threads: u32,
    /// The bytes lent buffers may occupy at once, across every call. Zero is no ceiling.
    pub memory_ceiling: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// How far along a channel's closing is. A handle the runtime no longer knows reads as `NONE`,
/// which is also what an unopened one reads as: neither names a channel.
pub enum ak_channel_state {
    AK_CHANNEL_NONE = 0,
    AK_CHANNEL_OPEN = 1,
    AK_CHANNEL_CLOSING = 2,
    AK_CHANNEL_CLOSED = 3,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
/// What the runtime-wide ceiling is holding.
///
/// Only a fall in the total proves capacity came back: committing a buffer hands the same bytes
/// from the host to the runtime rather than freeing anything.
pub struct ak_memory_usage {
    pub bytes_used: u64,
    pub ceiling: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
/// How a call is started.
pub struct ak_call_start_options {
    pub struct_size: u32,
    /// `/Service/Method`, not NUL-terminated.
    pub method: ak_bytes_in,
    /// The key/value blob [`crate::blob`] describes. May be empty.
    pub metadata: ak_bytes_in,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
/// What a call still owes, for a host that wants to assert on it.
pub struct ak_call_debt {
    pub payloads_owed: u32,
    pub buffers_lent: u32,
    pub callbacks_in_flight: u32,
    pub terminal_delivered: i32,
}

pub const AK_ABI_VERSION: i32 = 1;

/// Whether a struct the host built is one this ABI knows.
///
/// A size below the known one is a caller compiled against a version that did not have the
/// fields this one reads; a size above is one compiled against a version this library predates.
/// Neither can be read safely, so both are refused.
pub(crate) fn known_size<T>(struct_size: u32) -> bool {
    struct_size as usize == std::mem::size_of::<T>()
}
