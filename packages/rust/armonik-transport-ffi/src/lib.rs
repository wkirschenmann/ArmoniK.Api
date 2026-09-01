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
mod config;
mod host;
mod registry;
mod runtime;
mod tables;

use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use armonik_transport::grpc::{CallStartOptions, GrpcChannel, TokioExecutor};

pub use abi::*;
use call::Payload;
use host::{Host, HostPtr};
use runtime::{AkChannel, AkRuntime};

/// Runs `body`, answering `AK_STATUS_INTERNAL` if it panics.
fn guard(body: impl FnOnce() -> ak_status) -> ak_status {
    catch_unwind(AssertUnwindSafe(body)).unwrap_or(ak_status::AK_STATUS_INTERNAL)
}

// === Runtime lifecycle ===

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
        // SAFETY: the host's contract for this argument.
        let config = unsafe { *config };
        if !known_size::<ak_runtime_config>(config.struct_size) {
            return ak_status::AK_STATUS_INVALID_ARG;
        }

        match AkRuntime::new(
            config.worker_threads,
            config.memory_ceiling,
            Host::new(callback, runtime_ctx),
        ) {
            Err(status) => status,
            Ok(runtime) => {
                let handle = tables::runtimes().insert(runtime);
                // SAFETY: checked non-null above.
                unsafe { *out = handle };
                ak_status::AK_STATUS_OK
            }
        }
    })
}

/// What the runtime is doing. This, and no callback, is what permits destroying it.
#[no_mangle]
pub extern "C" fn ak_runtime_status(runtime: ak_handle) -> ak_runtime_state {
    catch_unwind(|| match tables::runtimes().get(runtime) {
        Some(runtime) => runtime.state(),
        // A token naming nothing names a runtime already destroyed, and destroying is legal only
        // from quiescence, so that is what it was.
        None => ak_runtime_state::AK_RUNTIME_QUIESCENT,
    })
    .unwrap_or(ak_runtime_state::AK_RUNTIME_FAILED_UNQUIESCED)
}

/// Closes the start gate and drains. Idempotent.
#[no_mangle]
pub extern "C" fn ak_runtime_begin_shutdown(runtime: ak_handle) -> ak_status {
    guard(|| match tables::runtimes().get(runtime) {
        None => ak_status::AK_STATUS_HANDLE_STALE,
        Some(runtime) => {
            runtime.begin_shutdown();
            ak_status::AK_STATUS_OK
        }
    })
}

/// Frees the runtime. Refused before quiescence, and that is the only reason.
#[no_mangle]
pub extern "C" fn ak_runtime_destroy(runtime: ak_handle) -> ak_status {
    guard(|| {
        let Some(found) = tables::runtimes().get(runtime) else {
            return ak_status::AK_STATUS_HANDLE_STALE;
        };
        if found.state() != ak_runtime_state::AK_RUNTIME_QUIESCENT {
            return ak_status::AK_STATUS_INVALID_STATE;
        }

        found.stale_own_handles();
        found.release_threads();
        tables::runtimes().remove(runtime);
        ak_status::AK_STATUS_OK
    })
}

// === Channel ===

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
        if !found.accepts_new_work() {
            return ak_status::AK_STATUS_INVALID_STATE;
        }
        // SAFETY: the host's contract for this argument.
        let Some(json) = (unsafe { config_json.as_slice() }) else {
            return ak_status::AK_STATUS_INVALID_ARG;
        };
        let Some(settings) = config::parse(json) else {
            return ak_status::AK_STATUS_INVALID_ARG;
        };

        let executor = TokioExecutor::new(found.spawner().clone());
        let Ok(grpc) = GrpcChannel::new(settings.into_channel_config(), executor) else {
            return ak_status::AK_STATUS_INVALID_ARG;
        };

        let channel = Arc::new(AkChannel::new(grpc, &found));
        let handle = tables::channels().insert(Arc::clone(&channel));
        channel.name_it(handle);
        // SAFETY: checked non-null above.
        unsafe { *out = handle };
        ak_status::AK_STATUS_OK
    })
}

/// Frees the channel. Calls under way are cancelled.
#[no_mangle]
pub extern "C" fn ak_channel_release(channel: ak_handle) {
    let _ = catch_unwind(|| {
        if let Some(found) = tables::channels().remove(channel) {
            found.grpc.close();
        }
    });
}

// === Call ===

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
        // SAFETY: the host's contract for this argument.
        let options = unsafe { *options };
        if !known_size::<ak_call_start_options>(options.struct_size) {
            return ak_status::AK_STATUS_INVALID_ARG;
        }

        let Some(found) = tables::channels().get(channel) else {
            return ak_status::AK_STATUS_HANDLE_STALE;
        };
        let Some(runtime) = found.runtime.upgrade() else {
            return ak_status::AK_STATUS_HANDLE_STALE;
        };
        if !runtime.accepts_new_work() {
            return ak_status::AK_STATUS_INVALID_STATE;
        }

        // SAFETY: the host's contract for these arguments.
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

        let mut start = CallStartOptions::new(method);
        start.metadata = metadata;
        let Ok(grpc_call) = found.grpc.start_call(start) else {
            return ak_status::AK_STATUS_INVALID_ARG;
        };

        let (send, recv, control) = grpc_call.split();
        let (state, commands) = call::create(
            HostPtr(call_ctx),
            &runtime,
            control,
            config::MAX_SENDS_IN_FLIGHT,
            config::DELIVERY_CREDITS,
        );
        let handle = tables::calls().insert(Arc::clone(&state));
        call::start(
            &state,
            handle,
            call::Halves { send, recv },
            commands,
            runtime.spawner(),
        );

        // SAFETY: checked non-null above.
        unsafe { *out = handle };
        ak_status::AK_STATUS_OK
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
        match found.lend(len) {
            Ok(buffer) => {
                // SAFETY: checked non-null above.
                unsafe { *out = buffer };
                ak_status::AK_STATUS_OK
            }
            // On a refusal nothing is lent and `out` stays as the host left it.
            Err(status) => status,
        }
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
        let Some(found) = tables::calls().get(call) else {
            return ak_status::AK_STATUS_HANDLE_STALE;
        };
        // SAFETY: forwarded from this function's own contract.
        unsafe { found.commit(buffer) }
    })
}

/// Gives a lent buffer back unused. Legal on a cancelled or terminal call.
///
/// # Safety
///
/// `buffer` must be one this call lent and the host has not given back.
#[no_mangle]
pub unsafe extern "C" fn ak_return_call_buffer(call: ak_handle, buffer: ak_buffer) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Some(found) = tables::calls().get(call) {
            // SAFETY: forwarded from this function's own contract.
            unsafe { found.give_back(buffer) }
        }
    }));
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
    guard(|| {
        if out.is_null() {
            return ak_status::AK_STATUS_INVALID_ARG;
        }
        let Some(found) = tables::calls().get(call) else {
            return ak_status::AK_STATUS_HANDLE_STALE;
        };
        // SAFETY: checked non-null above.
        unsafe { *out = found.debt() };
        ak_status::AK_STATUS_OK
    })
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
    guard(|| {
        if out.is_null() {
            return ak_status::AK_STATUS_INVALID_ARG;
        }
        let Some(found) = tables::runtimes().get(runtime) else {
            return ak_status::AK_STATUS_HANDLE_STALE;
        };
        // SAFETY: checked non-null above.
        unsafe { *out = found.ledger.usage() };
        ak_status::AK_STATUS_OK
    })
}

// === Utilities ===

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
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if payload.owner.is_null() {
            return;
        }
        // SAFETY: forwarded from this function's own contract. Dropping is what returns the
        // delivery credit, clears the call's debt and frees the bytes, so no path here can do
        // half of it.
        drop(unsafe { Box::from_raw(payload.owner as *mut Payload) });
    }));
}
