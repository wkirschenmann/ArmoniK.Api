use std::ffi::c_void;
use std::mem::{align_of, offset_of, size_of};

use armonik_transport_ffi::*;

const PTR: usize = size_of::<usize>();

#[test]
fn a_borrowed_view_is_a_pointer_and_a_length() {
    assert_eq!(size_of::<ak_bytes_in>(), 2 * PTR);
    assert_eq!(align_of::<ak_bytes_in>(), PTR);
    assert_eq!(offset_of!(ak_bytes_in, ptr), 0);
    assert_eq!(offset_of!(ak_bytes_in, len), PTR);
}

#[test]
fn an_owned_view_and_a_lent_buffer_have_the_same_shape() {
    assert_eq!(size_of::<ak_bytes>(), 3 * PTR);
    assert_eq!(offset_of!(ak_bytes, ptr), 0);
    assert_eq!(offset_of!(ak_bytes, len), PTR);
    assert_eq!(offset_of!(ak_bytes, owner), 2 * PTR);

    assert_eq!(size_of::<ak_buffer>(), 3 * PTR);
    assert_eq!(offset_of!(ak_buffer, ptr), 0);
    assert_eq!(offset_of!(ak_buffer, len), PTR);
    assert_eq!(offset_of!(ak_buffer, owner), 2 * PTR);
}

#[test]
fn an_event_carries_its_payload_inline() {
    assert_eq!(size_of::<ak_event>(), 4 * PTR + 8);
    assert_eq!(offset_of!(ak_event, kind), 0);
    assert_eq!(offset_of!(ak_event, payload), PTR);
    assert_eq!(offset_of!(ak_event, status_code), 4 * PTR);
    assert_eq!(offset_of!(ak_event, host_debt), 4 * PTR + 4);
}

#[test]
fn every_enum_the_abi_crosses_is_an_int() {
    assert_eq!(size_of::<ak_status>(), 4);
    assert_eq!(size_of::<ak_runtime_state>(), 4);
    assert_eq!(size_of::<ak_event_kind>(), 4);
    assert_eq!(size_of::<ak_host_debt>(), 4);
    assert_eq!(size_of::<ak_channel_state>(), 4);
}

#[test]
fn an_options_struct_starts_with_the_size_that_versions_it() {
    assert_eq!(offset_of!(ak_runtime_config, struct_size), 0);
    assert_eq!(offset_of!(ak_runtime_config, worker_threads), 4);
    assert_eq!(offset_of!(ak_runtime_config, memory_ceiling), 8);
    assert_eq!(size_of::<ak_runtime_config>(), 16);

    assert_eq!(offset_of!(ak_call_start_options, struct_size), 0);
    assert_eq!(offset_of!(ak_call_start_options, method), PTR);
    assert_eq!(offset_of!(ak_call_start_options, metadata), 3 * PTR);
    assert_eq!(size_of::<ak_call_start_options>(), 5 * PTR);
}

#[test]
fn the_observational_structs_are_plain_integers() {
    assert_eq!(size_of::<ak_call_debt>(), 16);
    assert_eq!(offset_of!(ak_call_debt, payloads_owed), 0);
    assert_eq!(offset_of!(ak_call_debt, buffers_lent), 4);
    assert_eq!(offset_of!(ak_call_debt, callbacks_in_flight), 8);
    assert_eq!(offset_of!(ak_call_debt, terminal_delivered), 12);

    assert_eq!(size_of::<ak_memory_usage>(), 16);
    assert_eq!(offset_of!(ak_memory_usage, bytes_used), 0);
    assert_eq!(offset_of!(ak_memory_usage, ceiling), 8);
}

#[test]
fn the_null_token_is_zero_so_a_zeroed_handle_names_nothing() {
    assert_eq!(AK_HANDLE_NONE, 0);
    assert_eq!(size_of::<ak_handle>(), 8);
}

fn header() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/include/armonik_transport_ffi.h"
    ))
    .expect("the header is committed beside the crate")
}

#[test]
fn the_header_and_the_library_agree_on_the_version() {
    let header = header();

    let declared = format!("#define AK_ABI_VERSION {AK_ABI_VERSION}");
    assert!(
        header.contains(&declared),
        "the header does not say {declared}"
    );
}

/// Each entry point coerced to the signature the header declares for it, written out here so a
/// parameter that changes type or moves is a compile error rather than nothing at all.
///
/// The list below only ever compared names: every function was cast to `*const ()`, which erases
/// exactly what a host binds against.
#[test]
fn every_entry_point_has_the_signature_the_header_declares() {
    let _: unsafe extern "C" fn(
        *const ak_runtime_config,
        Option<ak_callback>,
        *mut c_void,
        *mut ak_handle,
    ) -> ak_status = ak_runtime_create;
    let _: extern "C" fn(ak_handle) -> ak_runtime_state = ak_runtime_status;
    let _: extern "C" fn(ak_handle) -> ak_status = ak_runtime_begin_shutdown;
    let _: extern "C" fn(ak_handle) -> ak_status = ak_runtime_destroy;
    let _: unsafe extern "C" fn(ak_handle, *mut ak_memory_usage) -> ak_status =
        ak_runtime_memory_usage;

    let _: unsafe extern "C" fn(ak_handle, ak_bytes_in, *mut ak_handle) -> ak_status =
        ak_channel_create;
    let _: extern "C" fn(ak_handle) = ak_channel_release;
    let _: extern "C" fn(ak_handle) -> ak_channel_state = ak_channel_status;

    let _: unsafe extern "C" fn(
        ak_handle,
        *const ak_call_start_options,
        *mut c_void,
        *mut ak_handle,
    ) -> ak_status = ak_call_start;
    let _: unsafe extern "C" fn(ak_handle, usize, *mut ak_buffer) -> ak_status = ak_get_call_buffer;
    let _: unsafe extern "C" fn(ak_handle, ak_buffer) -> ak_status = ak_call_send_message;
    let _: unsafe extern "C" fn(ak_buffer) = ak_return_call_buffer;
    let _: extern "C" fn(ak_handle) -> ak_status = ak_call_end_send;
    let _: extern "C" fn(ak_handle) -> ak_status = ak_call_cancel;
    let _: unsafe extern "C" fn(ak_handle, *mut ak_call_debt) -> ak_status = ak_call_debt_of;

    let _: extern "C" fn() -> i32 = ak_abi_version;
    let _: unsafe extern "C" fn(ak_bytes) = ak_event_consumed;
}

#[test]
fn every_entry_point_the_header_declares_is_exported() {
    let header = header();

    let exported: &[(&str, *const ())] = &[
        ("ak_runtime_create", ak_runtime_create as *const ()),
        ("ak_runtime_status", ak_runtime_status as *const ()),
        (
            "ak_runtime_begin_shutdown",
            ak_runtime_begin_shutdown as *const (),
        ),
        ("ak_runtime_destroy", ak_runtime_destroy as *const ()),
        (
            "ak_runtime_memory_usage",
            ak_runtime_memory_usage as *const (),
        ),
        ("ak_channel_create", ak_channel_create as *const ()),
        ("ak_channel_release", ak_channel_release as *const ()),
        ("ak_channel_status", ak_channel_status as *const ()),
        ("ak_call_start", ak_call_start as *const ()),
        ("ak_get_call_buffer", ak_get_call_buffer as *const ()),
        ("ak_call_send_message", ak_call_send_message as *const ()),
        ("ak_return_call_buffer", ak_return_call_buffer as *const ()),
        ("ak_call_end_send", ak_call_end_send as *const ()),
        ("ak_call_cancel", ak_call_cancel as *const ()),
        ("ak_call_debt_of", ak_call_debt_of as *const ()),
        ("ak_abi_version", ak_abi_version as *const ()),
        ("ak_event_consumed", ak_event_consumed as *const ()),
    ];

    for (name, address) in exported {
        assert!(!address.is_null());
        assert!(
            header.contains(&format!("{name}(")),
            "{name} is exported and the header does not declare it"
        );
    }

    for line in header.lines() {
        let Some(rest) = [
            "ak_status ak_",
            "void ak_",
            "int ak_",
            "ak_runtime_state ak_",
            "ak_channel_state ak_",
        ]
        .iter()
        .find_map(|returns| line.strip_prefix(returns)) else {
            continue;
        };
        let name = format!("ak_{}", rest.split('(').next().unwrap_or_default());
        assert!(
            exported.iter().any(|(known, _)| *known == name),
            "the header declares {name}, which this library does not export"
        );
    }
}
