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
use host::{Host, HostPtr};
use runtime::{AkChannel, AkRuntime};

fn guard(body: impl FnOnce() -> ak_status) -> ak_status {
    catch_unwind(AssertUnwindSafe(body)).unwrap_or(ak_status::AK_STATUS_INTERNAL)
}

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

        match AkRuntime::new(
            config.worker_threads,
            config.memory_ceiling,
            Host::new(callback, runtime_ctx),
        ) {
            Err(status) => status,
            Ok(runtime) => {
                let handle = tables::runtimes().reserve();
                tables::runtimes().publish(handle, runtime);
                unsafe { *out = handle };
                ak_status::AK_STATUS_OK
            }
        }
    })
}

#[no_mangle]
pub extern "C" fn ak_runtime_status(runtime: ak_handle) -> ak_runtime_state {
    catch_unwind(|| match tables::runtimes().get(runtime) {
        Some(runtime) => runtime.state(),
        None => ak_runtime_state::AK_RUNTIME_QUIESCENT,
    })
    .unwrap_or(ak_runtime_state::AK_RUNTIME_FAILED_UNQUIESCED)
}

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
        let Some(settings) = config::parse(json) else {
            return ak_status::AK_STATUS_INVALID_ARG;
        };

        let executor = TokioExecutor::new(found.spawner().clone());
        let Ok(grpc) = GrpcChannel::new(settings.into_channel_config(), executor) else {
            return ak_status::AK_STATUS_INVALID_ARG;
        };

        let handle = tables::channels().reserve();
        let channel = Arc::new(AkChannel::new(grpc, &found, handle));
        tables::channels().publish(handle, channel);
        unsafe { *out = handle };
        ak_status::AK_STATUS_OK
    })
}

#[no_mangle]
pub extern "C" fn ak_channel_release(channel: ak_handle) {
    let _ = catch_unwind(|| {
        if let Some(found) = tables::channels().remove(channel) {
            found.grpc.close();
        }
    });
}

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
        let Some(runtime) = found.runtime.upgrade() else {
            return ak_status::AK_STATUS_HANDLE_STALE;
        };
        let Some(_pass) = runtime.pass_the_gate() else {
            return ak_status::AK_STATUS_INVALID_STATE;
        };

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
        let handle = tables::calls().reserve();
        let (state, commands) = call::create(
            HostPtr(call_ctx),
            handle,
            &runtime,
            control,
            config::MAX_SENDS_IN_FLIGHT,
            call::DELIVERY_CREDITS,
        );
        tables::calls().publish(handle, Arc::clone(&state));
        call::start(&state, send, recv, commands, runtime.spawner());

        unsafe { *out = handle };
        ak_status::AK_STATUS_OK
    })
}

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
                unsafe { *out = buffer };
                ak_status::AK_STATUS_OK
            }
            Err(status) => status,
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn ak_call_send_message(call: ak_handle, buffer: ak_buffer) -> ak_status {
    guard(|| {
        let Some(lent) = (unsafe { call::take_lent(buffer.owner) }) else {
            return ak_status::AK_STATUS_INVALID_ARG;
        };
        let (Some(owner), Some(found)) = (lent.call(), tables::calls().get(call)) else {
            let _ = Box::into_raw(lent);
            return ak_status::AK_STATUS_HANDLE_STALE;
        };
        if !Arc::ptr_eq(&owner, &found) {
            let _ = Box::into_raw(lent);
            return ak_status::AK_STATUS_INVALID_ARG;
        }
        found.commit(lent)
    })
}

#[no_mangle]
pub unsafe extern "C" fn ak_return_call_buffer(buffer: ak_buffer) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let Some(lent) = (unsafe { call::take_lent(buffer.owner) }) else {
            return;
        };
        match lent.call() {
            Some(call) => call.give_back(lent),
            None => drop(lent),
        }
    }));
}

#[no_mangle]
pub extern "C" fn ak_call_end_send(call: ak_handle) -> ak_status {
    guard(|| match tables::calls().get(call) {
        None => ak_status::AK_STATUS_HANDLE_STALE,
        Some(found) => found.end_send(),
    })
}

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

#[no_mangle]
pub unsafe extern "C" fn ak_call_debt_of(call: ak_handle, out: *mut ak_call_debt) -> ak_status {
    guard(|| {
        if out.is_null() {
            return ak_status::AK_STATUS_INVALID_ARG;
        }
        let Some(found) = tables::calls().get(call) else {
            return ak_status::AK_STATUS_HANDLE_STALE;
        };
        unsafe { *out = found.debt() };
        ak_status::AK_STATUS_OK
    })
}

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
        unsafe { *out = found.ledger.usage() };
        ak_status::AK_STATUS_OK
    })
}

#[no_mangle]
pub extern "C" fn ak_abi_version() -> i32 {
    AK_ABI_VERSION
}

#[no_mangle]
pub unsafe extern "C" fn ak_event_consumed(payload: ak_bytes) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if payload.owner.is_null() {
            return;
        }
        drop(unsafe { call::take_payload(payload.owner) });
    }));
}
