//! The C ABI's types, as Rust sees them.
//!
//! `include/armonik_transport_ffi.h` is rendered from this file and `lib.rs`, doc comments
//! included, so what an item says here is what a C host reads about it. Nothing here changes
//! without the header changing with it, and once the ABI is published `ak_abi_version` too: a
//! binding built against the old header would keep loading, and read the wrong bytes. Until then
//! no host outside this repository is compiled against it, which is why the version stays 1
//! (tasks.md, T4.0).

use std::ffi::c_void;

use armonik_transport::grpc::HeadOrigin;

/// Returned by every entry point that can fail. ak_event_consumed, ak_return_call_buffer and
/// ak_channel_release are void because a wrong token is a host bug the ABI cannot report anywhere
/// useful; ak_runtime_status, ak_channel_status and ak_abi_version return their answer.
#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ak_status {
    AK_STATUS_OK = 0,
    /// The object is gone; the token names nothing.
    AK_STATUS_HANDLE_STALE = 1,
    /// This call's send window is full - backpressure, not an error; retry when a WRITE_DONE
    /// arrives. That acquittal says this library has taken the message and the host's buffer is
    /// free; it says nothing about the peer, and does not wait on one - the wake-up is promised,
    /// its timing is not.
    AK_STATUS_SLOT_BUSY = 2,
    /// A null pointer, or a struct whose size prefix does not match any known version.
    AK_STATUS_INVALID_ARG = 3,
    /// A fault the ABI cannot attribute.
    AK_STATUS_INTERNAL = 4,
    /// The runtime-wide byte ceiling is reached - poll ak_runtime_memory_usage and retry.
    AK_STATUS_BUDGET_BUSY = 5,
    /// A valid handle at the wrong moment: a send after the terminal, a start while stopping, a
    /// destroy before quiescence. A guard refused, which is not a fault.
    AK_STATUS_INVALID_STATE = 6,
    /// The length exceeds the ceiling itself, so no return by anyone will ever make room.
    /// Permanent; do not retry.
    AK_STATUS_MESSAGE_TOO_LARGE = 7,
}

/// Only QUIESCENT permits ak_runtime_destroy or unloading the library. The host reaches it by
/// acting, not by waiting: while it still holds something the status stays STOPPED, and the
/// AK_EVENT_SHUTDOWN_COMPLETE callback says so through its host_debt field. Polling for QUIESCENT
/// before returning what it holds is therefore a deadlock.
///
/// NONE is what keeps a handle a token rather than a pointer: a handle this library does not
/// know - never created, or reclaimed by ak_runtime_destroy - is reported, not resolved.
/// It is not QUIESCENT, which would tell a host that passed a channel handle by mistake that it
/// may destroy the runtime and unload the library while one is running.
#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ak_runtime_state {
    /// This library knows no such runtime.
    AK_RUNTIME_NONE = 0,
    /// Operational, accepts channels and calls.
    AK_RUNTIME_RUNNING = 1,
    /// Start gate closed, channels closing.
    AK_RUNTIME_GRPC_STOPPING = 2,
    /// The gRPC side is done; the host may still hold memory.
    AK_RUNTIME_GRPC_STOPPED = 3,
    /// And nothing of it is outstanding either.
    AK_RUNTIME_QUIESCENT = 4,
    /// Quiescence impossible, destroy refused. Reported when the status itself cannot be read,
    /// when the task driving the shutdown dies, and when that shutdown cannot get a thread of its
    /// own: the three ways the step to QUIESCENT stops being reachable. A shutdown that is merely
    /// slow stays GRPC_STOPPING.
    AK_RUNTIME_FAILED_UNQUIESCED = 5,
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ak_event_kind {
    /// payload = metadata blob (owned), status_code = its origin.
    AK_EVENT_INITIAL_METADATA = 1,
    /// payload = message bytes (owned).
    AK_EVENT_MESSAGE = 2,
    /// Terminal - payload = reason + trailers (owned).
    AK_EVENT_STATUS = 3,
    /// An accepted send is settled; its slot is already free.
    AK_EVENT_WRITE_DONE = 4,
    /// The runtime has stopped running.
    AK_EVENT_SHUTDOWN_COMPLETE = 5,
    /// And now nothing of it is outstanding.
    AK_EVENT_RESOURCES_RELEASED = 6,
}

/// Carried by AK_EVENT_INITIAL_METADATA in status_code: where the head came from. The event comes
/// first on every call, and its metadata is empty unless the peer's headers were delivered. Zero
/// is the peer's headers, so a host that ignores the field takes every head for the peer's.
#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ak_head_origin {
    /// The peer's response headers.
    AK_HEAD_RECEIVED = 0,
    /// A response came and delivered no head - the Trailers-Only shape, or an answer refused
    /// before its body; the terminal is the call's status, the peer's unless the call was stopped
    /// here first.
    AK_HEAD_TRAILERS_ONLY = 1,
    /// No response reached the call.
    AK_HEAD_NO_RESPONSE = 2,
}

impl From<HeadOrigin> for ak_head_origin {
    fn from(origin: HeadOrigin) -> Self {
        match origin {
            HeadOrigin::Wire => Self::AK_HEAD_RECEIVED,
            HeadOrigin::TrailersOnly => Self::AK_HEAD_TRAILERS_ONLY,
            HeadOrigin::NoResponse => Self::AK_HEAD_NO_RESPONSE,
        }
    }
}

/// Carried by AK_EVENT_SHUTDOWN_COMPLETE and by nothing else.
#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ak_host_debt {
    /// Nothing of ours is in your hands; no second event.
    AK_HOST_NOTHING_TO_RETURN = 0,
    /// Consume the payloads, return the buffers; AK_EVENT_RESOURCES_RELEASED follows.
    AK_HOST_MUST_RETURN = 1,
}

impl ak_runtime_state {
    pub(crate) fn from_repr(value: i32) -> Option<Self> {
        [
            Self::AK_RUNTIME_NONE,
            Self::AK_RUNTIME_RUNNING,
            Self::AK_RUNTIME_GRPC_STOPPING,
            Self::AK_RUNTIME_GRPC_STOPPED,
            Self::AK_RUNTIME_QUIESCENT,
            Self::AK_RUNTIME_FAILED_UNQUIESCED,
        ]
        .into_iter()
        .find(|state| *state as i32 == value)
    }
}

pub type ak_handle = u64;

pub const AK_HANDLE_NONE: ak_handle = 0 as ak_handle;

/// Chosen by the host, returned in each callback for its call. Opaque: never dereferenced here.
/// Must stay valid until the call's terminal (AK_EVENT_STATUS).
pub type ak_call_ctx = *mut c_void;

/// Bytes the host lends this library for the duration of one downcall.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ak_bytes_in {
    pub ptr: *const u8,
    pub len: usize,
}

impl ak_bytes_in {
    /// The lifetime is the borrow's, so nothing built from this outlives the downcall that was
    /// handed the pointer - which is as long as the host promises it is there.
    pub(crate) unsafe fn as_slice(&self) -> Option<&[u8]> {
        if self.len == 0 {
            return Some(&[]);
        }
        if self.ptr.is_null() {
            return None;
        }
        Some(unsafe { std::slice::from_raw_parts(self.ptr, self.len) })
    }
}

/// Owned by the host after reception, until ak_event_consumed.
///
/// owner, not ptr, identifies the allocation: the view may be a slice of a larger one. owner ==
/// NULL means unowned, never "empty" - a zero-length payload that took a delivery credit carries a
/// real owner and MUST be consumed, because the credit comes back with the acquittal and not with
/// the bytes.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ak_bytes {
    pub ptr: *const u8,
    pub len: usize,
    pub owner: *mut c_void,
}

impl ak_bytes {
    pub(crate) fn none() -> Self {
        Self {
            ptr: std::ptr::null(),
            len: 0,
            owner: std::ptr::null_mut(),
        }
    }
}

/// Lent by ak_get_call_buffer out of the call's arena. The host writes len bytes and gives it back
/// exactly once, by ak_call_send_message or ak_return_call_buffer. This library never reclaims a
/// lent buffer on its own - not on cancellation, not on channel close - which is what removes the
/// race between a writing thread and a cancelling one.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ak_buffer {
    pub ptr: *mut u8,
    pub len: usize,
    pub owner: *mut c_void,
}

/// On the callback's stack; no lifecycle of its own. The payload is owned when owner is not NULL.
///
/// WRITE_DONE, SHUTDOWN_COMPLETE and RESOURCES_RELEASED carry no payload: ptr == NULL, len == 0,
/// owner == NULL. ak_event_consumed on such a payload is a no-op, so a host may route every event
/// through one path.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ak_event {
    pub kind: ak_event_kind,
    pub payload: ak_bytes,
    /// gRPC status for AK_EVENT_STATUS, ak_head_origin for AK_EVENT_INITIAL_METADATA, zero on
    /// every other event.
    pub status_code: i32,
    /// AK_EVENT_SHUTDOWN_COMPLETE only.
    pub host_debt: ak_host_debt,
}

/// The function pointer must stay valid for the runtime's lifetime, and runtime_ctx until the
/// runtime's last event.
///
/// Data callbacks (INITIAL_METADATA, MESSAGE, STATUS) are serialized per call and concurrent
/// between calls. WRITE_DONE may arrive in parallel with any of them, including for the same call:
/// a per-call lock in the handler would hold the slot release hostage behind a slow message
/// handler.
///
/// A callback runs on one of this library's own worker threads, and every promise made here about
/// progress - a send reaching the wire, an acquittal, a call being reclaimed, a shutdown
/// completing - is made on those threads. So the callback must publish and return: record the
/// event where the host's own thread will find it, hand the payload on, and end. It must not
/// parse, allocate what it could have allocated earlier, take a lock the host's own code holds,
/// or run application code. A host that blocks here does not slow itself down; it stops the
/// library, and the guarantees above stop with it.
pub type ak_callback = Option<
    unsafe extern "C" fn(runtime_ctx: *mut c_void, call_ctx: ak_call_ctx, event: *const ak_event),
>;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ak_runtime_config {
    pub struct_size: u32,
    /// Zero leaves the choice to the runtime; at most AK_MAX_WORKER_THREADS.
    pub worker_threads: u32,
    /// Bytes lent buffers may occupy at once. Zero, or more than this library can lend, asks for
    /// its own ceiling: four gigabytes, or half the address space where that is smaller - the gRPC
    /// length prefix and the allocator between them admit no more, so a budget above it is one no
    /// single lend could draw on.
    pub memory_ceiling: u64,
}

/// How far along a channel's closing is. A handle this library no longer knows reads as NONE,
/// which is also what an unopened one reads as: neither names a channel.
#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ak_channel_state {
    AK_CHANNEL_NONE = 0,
    /// Takes calls.
    AK_CHANNEL_OPEN = 1,
    /// Released, its calls draining.
    AK_CHANNEL_CLOSING = 2,
    /// And nothing of it is active any more.
    AK_CHANNEL_CLOSED = 3,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ak_memory_usage {
    /// Occupied against the ceiling, atomic snapshot.
    pub bytes_used: u64,
    /// The limit in force, which is what was configured or this library's own where that is
    /// smaller. Never zero.
    pub ceiling: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ak_call_start_options {
    pub struct_size: u32,
    /// "/Service/Method", not NUL-terminated.
    pub method: ak_bytes_in,
    /// A metadata blob - a uint32_t count, then each key and each value as a uint32_t length and
    /// its bytes; may be empty.
    pub metadata: ak_bytes_in,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ak_call_debt {
    /// Delivered, not yet ak_event_consumed.
    pub payloads_owed: u32,
    /// Out of the arena, not yet given back.
    pub buffers_lent: u32,
    /// The runtime's own, informational.
    pub callbacks_in_flight: u32,
    /// 0 or 1.
    pub terminal_delivered: i32,
}

/// Why a fallible entry point refused, beyond what its status says.
#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ak_error_kind {
    /// No family: the status says what happened - backpressure, a request too large for the
    /// ceiling, a fault this library cannot attribute. Zero, so a zero-initialized ak_error reads
    /// as nothing more to say.
    AK_ERROR_NONE = 0,
    /// The configuration document or the endpoint, before any socket.
    AK_ERROR_CONFIG = 1,
    /// DNS, TCP, TLS handshake.
    AK_ERROR_CONNECTION = 2,
    /// HTTP/2 or gRPC framing, after a connection.
    AK_ERROR_TRANSPORT = 3,
    AK_ERROR_TIMEOUT = 4,
    AK_ERROR_CANCELLED = 5,
    /// The host used the ABI in a way it does not admit: a null pointer, a handle that names
    /// nothing, a downcall at a moment its object refuses it.
    AK_ERROR_USAGE = 6,
}

/// Filled by this library, read by the host, and written only when the status is not
/// AK_STATUS_OK. Fixed layout, with no size prefix: ak_abi_version() is the agreement.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ak_error {
    pub kind: ak_error_kind,
    /// UTF-8, the cause chain flattened into one message. detail.owner == NULL means there is
    /// nothing to free. Released by ak_error_release, never by ak_event_consumed.
    pub detail: ak_bytes,
}

pub const AK_ABI_VERSION: i32 = 1;

/// The largest worker_threads a runtime is created with; above it, AK_STATUS_INVALID_ARG.
///
/// A worker is an OS thread, so the useful range is the machine's core count and this is far past
/// any of them. The bound exists because the failures past it are not reportable: the runtime
/// sizes a per-worker table before creating anything, and near UINT32_MAX that allocation ends the
/// process - the one failure no status can report.
pub const AK_MAX_WORKER_THREADS: u32 = 1024;

/// Reads an options struct, once its size prefix says the host built it with these fields.
///
/// The prefix first, and only then the rest. The header promises a struct of an unknown size is
/// "refused rather than read", and a host built against a smaller definition of one is the case
/// that promise exists for: copying `size_of::<T>()` bytes out of it before looking would read
/// past the end of the caller's object.
///
/// `struct_size` at offset zero is what `tests/layout.rs` pins, which is what makes the cast to
/// `u32` sound for any version of any of these structs.
///
/// Both reads are unaligned. Alignment is the caller's `T`, not this one: a host built against a
/// version without the `u64` passes a pointer aligned to four, and asking it for eight would be
/// asking it to know a definition it does not have. These are plain data, so an unaligned read is
/// a copy either way.
///
/// # Safety
///
/// `at` must be non-null and readable for four bytes; and, when those four say
/// `size_of::<T>()`, readable for `size_of::<T>()`.
pub(crate) unsafe fn read_versioned<T: Copy>(at: *const T) -> Option<T> {
    let struct_size = unsafe { at.cast::<u32>().read_unaligned() };
    (struct_size as usize == std::mem::size_of::<T>()).then(|| unsafe { at.read_unaligned() })
}
