//! The layouts the header declares, checked against the ones this library compiles to.
//!
//! Nothing compiles `include/armonik_transport_ffi.h` in this workspace, so the header and the
//! Rust types could drift apart with nothing noticing. These are what a C compiler produces for
//! that header, expressed in pointer widths rather than in the numbers of one target: the ABI
//! ships for x86, x64 and arm64, and absolute sizes would only ever have described one of them.
//!
//! It is not the conformance test the ABI owes - that one compiles the header and links against
//! the built library - but it catches the failure that costs the most: a field added on one side
//! and forgotten on the other.

use std::mem::{align_of, offset_of, size_of};

use armonik_transport_ffi::*;

/// A pointer's width here. Every layout below is written in terms of it, so one set of
/// assertions covers a 32-bit target and a 64-bit one.
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
    // The payload is pointer-aligned, so on a 64-bit target the four-byte kind is followed by
    // four of padding and on a 32-bit one by none. Two four-byte enums then close it, which is
    // why the total is the same expression either way.
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
    // Eight on every target, pointers included: a handle is a generational token and not an
    // address, which is why nothing here may store one in a pointer-sized field.
    assert_eq!(size_of::<ak_handle>(), 8);
}

/// The header, which is the contract these tests check the library against.
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

#[test]
fn every_entry_point_the_header_declares_is_exported() {
    let header = header();

    // Taking the address is what proves the symbol exists with that signature; a name in the
    // header and nothing behind it is the failure this catches.
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

    // And the other way round: a declaration the library does not export.
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
