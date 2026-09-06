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

fn guard_with<T>(fallback: T, body: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(body)).unwrap_or(fallback)
}

fn guard(body: impl FnOnce() -> ak_status) -> ak_status {
    guard_with(ak_status::AK_STATUS_INTERNAL, body)
}

fn guard_void(body: impl FnOnce()) {
    let _ = catch_unwind(AssertUnwindSafe(body));
}

/// # Safety
///
/// `out` must be writable for its type.
unsafe fn hand_over<T>(out: *mut T, made: Result<T, ak_status>) -> ak_status {
    match made {
        Err(status) => status,
        Ok(value) => {
            unsafe { *out = value };
            ak_status::AK_STATUS_OK
        }
    }
}

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
    unsafe { *out = read(&found) };
    ak_status::AK_STATUS_OK
}

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
        let Some(config) = (unsafe { read_versioned(config) }) else {
            return ak_status::AK_STATUS_INVALID_ARG;
        };

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

#[no_mangle]
pub extern "C" fn ak_runtime_status(runtime: ak_handle) -> ak_runtime_state {
    guard_with(
        ak_runtime_state::AK_RUNTIME_FAILED_UNQUIESCED,
        || match tables::runtimes().get(runtime) {
            Some(runtime) => runtime.state(),
            None => ak_runtime_state::AK_RUNTIME_NONE,
        },
    )
}

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

#[no_mangle]
pub extern "C" fn ak_runtime_destroy(runtime: ak_handle) -> ak_status {
    guard(|| lifecycle::destroy_runtime(runtime))
}

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

#[no_mangle]
pub extern "C" fn ak_channel_release(channel: ak_handle) {
    guard_void(|| lifecycle::release_channel(channel));
}

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
        let Some(options) = (unsafe { read_versioned(options) }) else {
            return ak_status::AK_STATUS_INVALID_ARG;
        };

        let Some(found) = tables::channels().get(channel) else {
            return ak_status::AK_STATUS_HANDLE_STALE;
        };
        let Some(runtime) = tables::runtimes().get(found.runtime) else {
            return ak_status::AK_STATUS_HANDLE_STALE;
        };
        let Some(_pass) = runtime.pass_the_gate() else {
            return ak_status::AK_STATUS_INVALID_STATE;
        };
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
                    &runtime.services(),
                    method,
                    metadata,
                    HostPtr(call_ctx),
                ),
            )
        }
    })
}

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

/// # Safety
///
/// `buffer` must be one this call lent and the host has not given back.
#[no_mangle]
pub unsafe extern "C" fn ak_call_send_message(call: ak_handle, buffer: ak_buffer) -> ak_status {
    guard(|| {
        let Some(lent) = (unsafe { call::take_lent(buffer.owner) }) else {
            return ak_status::AK_STATUS_INVALID_ARG;
        };
        let Some(found) = tables::calls().get(call) else {
            return call::keep(lent, ak_status::AK_STATUS_HANDLE_STALE);
        };
        // The buffer names its own call, so a handle that names another one is the host's
        // mistake and not this library's to resolve.
        if !Arc::ptr_eq(lent.call(), &found) {
            return call::keep(lent, ak_status::AK_STATUS_INVALID_ARG);
        }
        found.commit(lent)
    })
}

/// # Safety
///
/// `buffer` must be one this call lent and the host has not given back.
#[no_mangle]
pub unsafe extern "C" fn ak_return_call_buffer(buffer: ak_buffer) {
    guard_void(|| {
        let Some(lent) = (unsafe { call::take_lent(buffer.owner) }) else {
            return;
        };
        Arc::clone(lent.call()).give_back(lent);
    });
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

/// # Safety
///
/// `out` must be writable.
#[no_mangle]
pub unsafe extern "C" fn ak_call_debt_of(call: ak_handle, out: *mut ak_call_debt) -> ak_status {
    guard(|| unsafe { observe(tables::calls(), call, out, |found| found.debt()) })
}

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

#[no_mangle]
pub extern "C" fn ak_abi_version() -> i32 {
    AK_ABI_VERSION
}

/// # Safety
///
/// `payload` must be one this library delivered and the host has not consumed.
#[no_mangle]
pub unsafe extern "C" fn ak_event_consumed(payload: ak_bytes) {
    guard_void(|| {
        drop(unsafe { call::take_payload(payload.owner) });
    });
}
