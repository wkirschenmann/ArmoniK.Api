//! The C ABI's types, as Rust sees them.
//!
//! Every discriminant, field order and size here is `include/armonik_transport_ffi.h`. Nothing in
//! this file may be changed without changing that header and `ak_abi_version` with it: a binding
//! built against the old header would keep loading, and read the wrong bytes.

use std::ffi::c_void;

use armonik_transport::grpc::ChannelError;

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
pub enum ak_runtime_state {
    AK_RUNTIME_RUNNING = 1,
    AK_RUNTIME_GRPC_STOPPING = 2,
    AK_RUNTIME_GRPC_STOPPED = 3,
    AK_RUNTIME_QUIESCENT = 4,
    AK_RUNTIME_FAILED_UNQUIESCED = 5,
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
pub enum ak_host_debt {
    AK_HOST_NOTHING_TO_RETURN = 0,
    AK_HOST_MUST_RETURN = 1,
}

impl ak_runtime_state {
    pub(crate) fn from_repr(value: i32) -> Option<Self> {
        [
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

pub const AK_HANDLE_NONE: ak_handle = 0;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ak_bytes_in {
    pub ptr: *const u8,
    pub len: usize,
}

impl ak_bytes_in {
    pub(crate) unsafe fn as_slice<'a>(&self) -> Option<&'a [u8]> {
        if self.len == 0 {
            return Some(&[]);
        }
        if self.ptr.is_null() {
            return None;
        }
        Some(unsafe { std::slice::from_raw_parts(self.ptr, self.len) })
    }
}

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

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ak_buffer {
    pub ptr: *mut u8,
    pub len: usize,
    pub owner: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ak_event {
    pub kind: ak_event_kind,
    pub payload: ak_bytes,
    pub status_code: i32,
    pub host_debt: ak_host_debt,
}

pub type ak_callback =
    unsafe extern "C" fn(runtime_ctx: *mut c_void, call_ctx: *mut c_void, event: *const ak_event);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ak_runtime_config {
    pub struct_size: u32,
    pub worker_threads: u32,
    pub memory_ceiling: u64,
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ak_channel_state {
    AK_CHANNEL_NONE = 0,
    AK_CHANNEL_OPEN = 1,
    AK_CHANNEL_CLOSING = 2,
    AK_CHANNEL_CLOSED = 3,
}

impl ak_channel_state {
    pub(crate) fn from_repr(value: i32) -> Option<Self> {
        [
            Self::AK_CHANNEL_NONE,
            Self::AK_CHANNEL_OPEN,
            Self::AK_CHANNEL_CLOSING,
            Self::AK_CHANNEL_CLOSED,
        ]
        .into_iter()
        .find(|state| *state as i32 == value)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ak_memory_usage {
    pub bytes_used: u64,
    pub ceiling: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ak_call_start_options {
    pub struct_size: u32,
    pub method: ak_bytes_in,
    pub metadata: ak_bytes_in,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ak_call_debt {
    pub payloads_owed: u32,
    pub buffers_lent: u32,
    pub callbacks_in_flight: u32,
    pub terminal_delivered: i32,
}

impl From<ChannelError> for ak_status {
    fn from(error: ChannelError) -> Self {
        match error {
            ChannelError::Closed => Self::AK_STATUS_INVALID_STATE,
            _ => Self::AK_STATUS_INVALID_ARG,
        }
    }
}

pub const AK_ABI_VERSION: i32 = 1;

pub(crate) fn known_size<T>(struct_size: u32) -> bool {
    struct_size as usize == std::mem::size_of::<T>()
}
