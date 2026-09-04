//! C ABI over `armonik-transport`'s gRPC engine, for hosts that cannot link Rust.
//!
//! Every entry point is a downcall: it does what it can synchronously and returns. What takes
//! time happens on the runtime's own threads and reaches the host through the callback given at
//! `ak_runtime_create`. `include/armonik_transport_ffi.h` is the contract; the reasoning behind
//! it is in `spec/armonik_grpc_ffi/design.md`.
//!
//! Two rules run through all of it. A handle is a token and not a pointer, so a downcall on
//! something the runtime has already reclaimed reports a status instead of faulting. And no panic
//! crosses the boundary: unwinding into C is undefined, so every entry point catches one.

#![allow(non_camel_case_types)]

mod abi;
mod blob;
mod call;
mod channel;
mod config;
mod host;
mod ledger;
mod lifecycle;
mod registry;
mod runtime;
mod tables;

use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

pub use abi::*;
use host::{Host, HostPtr};
use registry::Registry;

/// Runs `body`, answering `fallback` if it panics.
///
/// The three shapes below are the whole of "no panic crosses the boundary": an entry point that
/// answers a status, one that answers something else, and one that answers nothing. A new entry
/// point picks one rather than spelling the rule again.
fn guard_with<T>(fallback: T, body: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(body)).unwrap_or(fallback)
}

/// Runs `body`, answering `AK_STATUS_INTERNAL` if it panics.
fn guard(body: impl FnOnce() -> ak_status) -> ak_status {
    guard_with(ak_status::AK_STATUS_INTERNAL, body)
}

/// Runs `body`, which answers nothing, and swallows a panic.
fn guard_void(body: impl FnOnce()) {
    let _ = catch_unwind(AssertUnwindSafe(body));
}

/// Writes what was made into `out` and answers OK, or answers why it was not made.
///
/// On a refusal `*out` stays as the host left it, which is what the header promises of every
/// entry point that answers through one.
///
/// # Safety
///
/// `out` must be writable for its type.
unsafe fn hand_over<T>(out: *mut T, made: Result<T, ak_status>) -> ak_status {
    match made {
        Err(status) => status,
        Ok(value) => {
            // SAFETY: forwarded from this function's own contract.
            unsafe { *out = value };
            ak_status::AK_STATUS_OK
        }
    }
}

/// Writes what `read` says about the object `handle` names into `out`.
///
/// # Safety
///
/// `out` must be null or writable for its type.
unsafe fn observe<T, V>(
    table: &Registry<T>,
    handle: ak_handle,
    out: *mut V,
    read: impl FnOnce(&T) -> V,
) -> ak_status {
    if out.is_null() {
        return ak_status::AK_STATUS_INVALID_ARG;
    }
    let Some(found) = table.get(handle) else {
        return ak_status::AK_STATUS_HANDLE_STALE;
    };
    // SAFETY: forwarded from this function's own contract.
    unsafe { *out = read(&found) };
    ak_status::AK_STATUS_OK
}

/// Creates a runtime, which is running when this returns.
///
/// # Safety
///
/// `config` and `out` must be valid for their types, and `callback` must stay callable with
/// `runtime_ctx` until the runtime's last event.
#[no_mangle]
pub unsafe extern "C" fn ak_runtime_create(
    config: *const ak_runtime_config,
    callback: Option<ak_callback>,
    runtime_ctx: *mut c_void,
    out: *mut ak_handle,
) -> ak_status {
    guard(|| {
        let (Some(callback), false, false) = (callback, config.is_null(), out.is_null()) else {
            return ak_status::AK_STATUS_INVALID_ARG;
        };
        let config = unsafe { *config };
        if !known_size::<ak_runtime_config>(config.struct_size) {
            return ak_status::AK_STATUS_INVALID_ARG;
        }

        unsafe {
            hand_over(
                out,
                lifecycle::create_runtime(
                    config.worker_threads,
                    config.memory_ceiling,
                    Host::new(callback, runtime_ctx),
                ),
            )
        }
    })
}

/// How far along a channel's closing is, for a host that wants to watch the drain.
#[no_mangle]
pub extern "C" fn ak_channel_status(channel: ak_handle) -> ak_channel_state {
    guard_with(
        ak_channel_state::AK_CHANNEL_NONE,
        || match tables::channels().get(channel) {
            Some(found) => found.state(),
            None => ak_channel_state::AK_CHANNEL_NONE,
        },
    )
}

/// What the runtime is doing. This, and no callback, is what permits destroying it.
#[no_mangle]
pub extern "C" fn ak_runtime_status(runtime: ak_handle) -> ak_runtime_state {
    guard_with(ak_runtime_state::AK_RUNTIME_FAILED_UNQUIESCED, || {
        match tables::runtimes().get(runtime) {
            Some(runtime) => runtime.state(),
            // A token naming nothing names a runtime already destroyed, and destroying is legal
            // only from quiescence, so that is what it was.
            None => ak_runtime_state::AK_RUNTIME_QUIESCENT,
        }
    })
}

/// Closes the start gate and drains. Idempotent.
#[no_mangle]
pub extern "C" fn ak_runtime_begin_shutdown(runtime: ak_handle) -> ak_status {
    guard(|| match tables::runtimes().get(runtime) {
        None => ak_status::AK_STATUS_HANDLE_STALE,
        Some(runtime) => {
            lifecycle::begin_shutdown(&runtime);
            ak_status::AK_STATUS_OK
        }
    })
}

/// Frees the runtime. Refused before quiescence, and that is the only reason.
#[no_mangle]
pub extern "C" fn ak_runtime_destroy(runtime: ak_handle) -> ak_status {
    guard(|| lifecycle::destroy_runtime(runtime))
}

/// Creates a channel from a config JSON. Performs no I/O, so it fails only on a bad config.
///
/// # Safety
///
/// `config_json` must point at its bytes for the duration of the call, and `out` must be
/// writable.
#[no_mangle]
pub unsafe extern "C" fn ak_channel_create(
    runtime: ak_handle,
    config_json: ak_bytes_in,
    out: *mut ak_handle,
) -> ak_status {
    guard(|| {
        if out.is_null() {
            return ak_status::AK_STATUS_INVALID_ARG;
        }
        let Some(found) = tables::runtimes().get(runtime) else {
            return ak_status::AK_STATUS_HANDLE_STALE;
        };
        let Some(_pass) = found.pass_the_gate() else {
            return ak_status::AK_STATUS_INVALID_STATE;
        };
        let Some(json) = (unsafe { config_json.as_slice() }) else {
            return ak_status::AK_STATUS_INVALID_ARG;
        };

        unsafe { hand_over(out, channel::create(runtime, found.spawner(), json)) }
    })
}

/// Frees the channel, cancelling its calls first. The header says why the cancellation is this
/// library's own business rather than something the host must provoke.
#[no_mangle]
pub extern "C" fn ak_channel_release(channel: ak_handle) {
    guard_void(|| lifecycle::release_channel(channel));
}

/// Starts a call. `call_ctx` comes back in each of its events.
///
/// # Safety
///
/// `options` must be valid for its type and its byte views, and `out` must be writable.
#[no_mangle]
pub unsafe extern "C" fn ak_call_start(
    channel: ak_handle,
    options: *const ak_call_start_options,
    call_ctx: *mut c_void,
    out: *mut ak_handle,
) -> ak_status {
    guard(|| {
        if options.is_null() || out.is_null() {
            return ak_status::AK_STATUS_INVALID_ARG;
        }
        let options = unsafe { *options };
        if !known_size::<ak_call_start_options>(options.struct_size) {
            return ak_status::AK_STATUS_INVALID_ARG;
        }

        let Some(found) = tables::channels().get(channel) else {
            return ak_status::AK_STATUS_HANDLE_STALE;
        };
        let Some(runtime) = tables::runtimes().get(found.runtime) else {
            return ak_status::AK_STATUS_HANDLE_STALE;
        };
        let Some(_pass) = runtime.pass_the_gate() else {
            return ak_status::AK_STATUS_INVALID_STATE;
        };
        // Before the engine is asked: it refuses a closed session too, but as a bad argument,
        // and a channel that is on its way out is a state and not a malformed request.
        if found.state() != ak_channel_state::AK_CHANNEL_OPEN {
            return ak_status::AK_STATUS_INVALID_STATE;
        }

        let (Some(method), Some(metadata)) = (unsafe { options.method.as_slice() }, unsafe {
            options.metadata.as_slice()
        }) else {
            return ak_status::AK_STATUS_INVALID_ARG;
        };
        let (Ok(method), Some(metadata)) =
            (std::str::from_utf8(method), blob::decode_metadata(metadata))
        else {
            return ak_status::AK_STATUS_INVALID_ARG;
        };

        unsafe {
            hand_over(
                out,
                call::start_on(
                    &found,
                    channel,
                    &runtime.services(),
                    method,
                    metadata,
                    HostPtr(call_ctx),
                ),
            )
        }
    })
}

/// Lends a buffer out of the call's arena to serialize into.
///
/// # Safety
///
/// `out` must be writable.
#[no_mangle]
pub unsafe extern "C" fn ak_get_call_buffer(
    call: ak_handle,
    len: usize,
    out: *mut ak_buffer,
) -> ak_status {
    guard(|| {
        if out.is_null() {
            return ak_status::AK_STATUS_INVALID_ARG;
        }
        let Some(found) = tables::calls().get(call) else {
            return ak_status::AK_STATUS_HANDLE_STALE;
        };
        unsafe { hand_over(out, found.lend(len)) }
    })
}

/// Commits a lent buffer as the next message.
///
/// # Safety
///
/// `buffer` must be one this call lent and the host has not given back.
#[no_mangle]
pub unsafe extern "C" fn ak_call_send_message(call: ak_handle, buffer: ak_buffer) -> ak_status {
    guard(|| {
        let Some(lent) = (unsafe { call::take_lent(buffer.owner) }) else {
            return ak_status::AK_STATUS_INVALID_ARG;
        };
        let (Some(owner), Some(found)) = (lent.call(), tables::calls().get(call)) else {
            return call::keep(lent, ak_status::AK_STATUS_HANDLE_STALE);
        };
        // A buffer determines its call, so a pair that disagrees is a host bug rather than a
        // reason to move some other call's counters.
        if !Arc::ptr_eq(&owner, &found) {
            return call::keep(lent, ak_status::AK_STATUS_INVALID_ARG);
        }
        found.commit(lent)
    })
}

/// Gives a lent buffer back unused. Legal on a cancelled or terminal call.
///
/// Takes no call handle: a buffer determines its call, so there is no pair that can disagree and
/// no window in which the handle has gone stale and the memory has nowhere to go.
///
/// # Safety
///
/// `buffer` must be one this library lent and the host has not given back.
#[no_mangle]
pub unsafe extern "C" fn ak_return_call_buffer(buffer: ak_buffer) {
    guard_void(|| {
        let Some(lent) = (unsafe { call::take_lent(buffer.owner) }) else {
            return;
        };
        match lent.call() {
            Some(call) => call.give_back(lent),
            // The call is gone, so there are no counters left to move; the bytes still go.
            None => drop(lent),
        }
    });
}

/// Signals the end of sending. No message follows it.
#[no_mangle]
pub extern "C" fn ak_call_end_send(call: ak_handle) -> ak_status {
    guard(|| match tables::calls().get(call) {
        None => ak_status::AK_STATUS_HANDLE_STALE,
        Some(found) => found.end_send(),
    })
}

/// Cancels the call. The terminal that follows carries CANCELLED.
#[no_mangle]
pub extern "C" fn ak_call_cancel(call: ak_handle) -> ak_status {
    guard(|| match tables::calls().get(call) {
        None => ak_status::AK_STATUS_HANDLE_STALE,
        Some(found) => {
            found.cancel();
            ak_status::AK_STATUS_OK
        }
    })
}

/// What the call still owes. Purely observational.
///
/// # Safety
///
/// `out` must be writable.
#[no_mangle]
pub unsafe extern "C" fn ak_call_debt_of(call: ak_handle, out: *mut ak_call_debt) -> ak_status {
    guard(|| unsafe { observe(tables::calls(), call, out, |found| found.debt()) })
}

/// What the runtime-wide ceiling is holding. Purely observational.
///
/// # Safety
///
/// `out` must be writable.
#[no_mangle]
pub unsafe extern "C" fn ak_runtime_memory_usage(
    runtime: ak_handle,
    out: *mut ak_memory_usage,
) -> ak_status {
    guard(|| unsafe {
        observe(tables::runtimes(), runtime, out, |found| {
            found.ledger().usage()
        })
    })
}

/// The ABI version a binding compares against its own.
#[no_mangle]
pub extern "C" fn ak_abi_version() -> i32 {
    AK_ABI_VERSION
}

/// Frees a payload and arms the next event for its call.
///
/// # Safety
///
/// `payload` must be one this library delivered and the host has not consumed.
#[no_mangle]
pub unsafe extern "C" fn ak_event_consumed(payload: ak_bytes) {
    guard_void(|| {
        if payload.owner.is_null() {
            return;
        }
        // SAFETY: forwarded from this function's own contract. Dropping is what returns the
        // credit, clears the call's debt and frees the bytes, so no path can do half of it.
        drop(unsafe { call::take_payload(payload.owner) });
    });
}
