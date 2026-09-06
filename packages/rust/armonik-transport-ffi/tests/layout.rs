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

/// The type of every field the ABI crosses, which the offsets above do not pin.
///
/// A `len` narrowed to `u32` sits at the same offset, in a struct of the same size, behind the
/// padding a 64-bit target puts after it - every assertion above still holds, and a host writing
/// through `size_t` reads four bytes of whatever was on the stack. The coercion is what fails.
#[test]
fn every_field_has_the_type_the_header_declares() {
    let _: fn(&ak_bytes_in) -> &*const u8 = |view| &view.ptr;
    let _: fn(&ak_bytes_in) -> &usize = |view| &view.len;

    let _: fn(&ak_bytes) -> &*const u8 = |view| &view.ptr;
    let _: fn(&ak_bytes) -> &usize = |view| &view.len;
    let _: fn(&ak_bytes) -> &*mut c_void = |view| &view.owner;

    let _: fn(&ak_buffer) -> &*mut u8 = |buffer| &buffer.ptr;
    let _: fn(&ak_buffer) -> &usize = |buffer| &buffer.len;
    let _: fn(&ak_buffer) -> &*mut c_void = |buffer| &buffer.owner;

    let _: fn(&ak_event) -> &ak_event_kind = |event| &event.kind;
    let _: fn(&ak_event) -> &ak_bytes = |event| &event.payload;
    let _: fn(&ak_event) -> &i32 = |event| &event.status_code;
    let _: fn(&ak_event) -> &ak_host_debt = |event| &event.host_debt;

    let _: fn(&ak_runtime_config) -> &u32 = |config| &config.struct_size;
    let _: fn(&ak_runtime_config) -> &u32 = |config| &config.worker_threads;
    let _: fn(&ak_runtime_config) -> &u64 = |config| &config.memory_ceiling;

    let _: fn(&ak_call_start_options) -> &u32 = |options| &options.struct_size;
    let _: fn(&ak_call_start_options) -> &ak_bytes_in = |options| &options.method;
    let _: fn(&ak_call_start_options) -> &ak_bytes_in = |options| &options.metadata;

    let _: fn(&ak_call_debt) -> &u32 = |debt| &debt.payloads_owed;
    let _: fn(&ak_call_debt) -> &u32 = |debt| &debt.buffers_lent;
    let _: fn(&ak_call_debt) -> &u32 = |debt| &debt.callbacks_in_flight;
    let _: fn(&ak_call_debt) -> &i32 = |debt| &debt.terminal_delivered;

    let _: fn(&ak_memory_usage) -> &u64 = |usage| &usage.bytes_used;
    let _: fn(&ak_memory_usage) -> &u64 = |usage| &usage.ceiling;
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

/// Every `AK_...` constant the header gives a value to.
///
/// Only lines that start with the name, so a constant named in prose is not one of these.
fn header_constants(header: &str) -> Vec<(String, i32)> {
    let mut found = Vec::new();
    for line in without_comments(header).lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("AK_") else {
            continue;
        };
        let Some((name, value)) = rest.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_end_matches(',').trim();
        let Ok(value) = value.parse::<i32>() else {
            continue;
        };
        found.push((format!("AK_{}", name.trim()), value));
    }
    found
}

/// The header with its comments taken out, so what is scanned is what a compiler sees.
fn without_comments(header: &str) -> String {
    let mut kept = String::with_capacity(header.len());
    let mut rest = header;
    while let Some(open) = rest.find("/*") {
        kept.push_str(&rest[..open]);
        // A comment stands for a space, so a declaration split by one still reads as two tokens.
        kept.push(' ');
        let after = &rest[open + 2..];
        match after.find("*/") {
            Some(close) => rest = &after[close + 2..],
            None => return kept,
        }
    }
    kept.push_str(rest);
    kept
}

#[test]
fn every_enum_value_is_the_one_the_header_gives_it() {
    // Written out rather than derived: this list and the header are the two statements of the
    // ABI, and a test that read the values from the same place as the code would compare a thing
    // to itself.
    let library: &[(&str, i32)] = &[
        ("AK_STATUS_OK", ak_status::AK_STATUS_OK as i32),
        (
            "AK_STATUS_HANDLE_STALE",
            ak_status::AK_STATUS_HANDLE_STALE as i32,
        ),
        ("AK_STATUS_SLOT_BUSY", ak_status::AK_STATUS_SLOT_BUSY as i32),
        (
            "AK_STATUS_INVALID_ARG",
            ak_status::AK_STATUS_INVALID_ARG as i32,
        ),
        ("AK_STATUS_INTERNAL", ak_status::AK_STATUS_INTERNAL as i32),
        (
            "AK_STATUS_BUDGET_BUSY",
            ak_status::AK_STATUS_BUDGET_BUSY as i32,
        ),
        (
            "AK_STATUS_INVALID_STATE",
            ak_status::AK_STATUS_INVALID_STATE as i32,
        ),
        (
            "AK_STATUS_MESSAGE_TOO_LARGE",
            ak_status::AK_STATUS_MESSAGE_TOO_LARGE as i32,
        ),
        ("AK_RUNTIME_NONE", ak_runtime_state::AK_RUNTIME_NONE as i32),
        (
            "AK_RUNTIME_RUNNING",
            ak_runtime_state::AK_RUNTIME_RUNNING as i32,
        ),
        (
            "AK_RUNTIME_GRPC_STOPPING",
            ak_runtime_state::AK_RUNTIME_GRPC_STOPPING as i32,
        ),
        (
            "AK_RUNTIME_GRPC_STOPPED",
            ak_runtime_state::AK_RUNTIME_GRPC_STOPPED as i32,
        ),
        (
            "AK_RUNTIME_QUIESCENT",
            ak_runtime_state::AK_RUNTIME_QUIESCENT as i32,
        ),
        (
            "AK_RUNTIME_FAILED_UNQUIESCED",
            ak_runtime_state::AK_RUNTIME_FAILED_UNQUIESCED as i32,
        ),
        ("AK_CHANNEL_NONE", ak_channel_state::AK_CHANNEL_NONE as i32),
        ("AK_CHANNEL_OPEN", ak_channel_state::AK_CHANNEL_OPEN as i32),
        (
            "AK_CHANNEL_CLOSING",
            ak_channel_state::AK_CHANNEL_CLOSING as i32,
        ),
        (
            "AK_CHANNEL_CLOSED",
            ak_channel_state::AK_CHANNEL_CLOSED as i32,
        ),
        (
            "AK_EVENT_INITIAL_METADATA",
            ak_event_kind::AK_EVENT_INITIAL_METADATA as i32,
        ),
        ("AK_EVENT_MESSAGE", ak_event_kind::AK_EVENT_MESSAGE as i32),
        ("AK_EVENT_STATUS", ak_event_kind::AK_EVENT_STATUS as i32),
        (
            "AK_EVENT_WRITE_DONE",
            ak_event_kind::AK_EVENT_WRITE_DONE as i32,
        ),
        (
            "AK_EVENT_SHUTDOWN_COMPLETE",
            ak_event_kind::AK_EVENT_SHUTDOWN_COMPLETE as i32,
        ),
        (
            "AK_EVENT_RESOURCES_RELEASED",
            ak_event_kind::AK_EVENT_RESOURCES_RELEASED as i32,
        ),
        (
            "AK_HOST_NOTHING_TO_RETURN",
            ak_host_debt::AK_HOST_NOTHING_TO_RETURN as i32,
        ),
        (
            "AK_HOST_MUST_RETURN",
            ak_host_debt::AK_HOST_MUST_RETURN as i32,
        ),
    ];

    let declared = header_constants(&header());
    assert!(!declared.is_empty(), "the header declares no constant");

    for (name, value) in &declared {
        let Some((_, ours)) = library.iter().find(|(known, _)| known == name) else {
            panic!("the header declares {name}, which this library does not");
        };
        assert_eq!(ours, value, "{name}");
    }

    for (name, _) in library {
        assert!(
            declared.iter().any(|(given, _)| given == name),
            "{name} is a value of this library and the header gives it none"
        );
    }
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

    // The whole line, not a substring of it: `AK_ABI_VERSION 1` is a prefix of
    // `AK_ABI_VERSION 10`, so a bump on one side alone would have gone unread.
    let declared = format!("#define AK_ABI_VERSION {AK_ABI_VERSION}");
    assert!(
        header.lines().any(|line| line.trim() == declared),
        "the header does not say {declared}"
    );
}

/// Each entry point coerced to the signature the header declares for it, so a parameter that
/// changes type or moves is a compile error rather than nothing at all.
///
/// The list below compares names. A name is not what a host binds against.
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

    // Whatever it returns: a list of return types is a filter, and a declaration returning one
    // it does not name goes unread. A name followed by `(` is what a declaration is; comments go
    // first, so a name written in prose is not mistaken for one.
    for name in header_declarations(&header) {
        assert!(
            exported.iter().any(|(known, _)| *known == name),
            "the header declares {name}, which this library does not export"
        );
    }
}

/// Every `ak_...(` in the header, which is every function it declares.
///
/// `ak_callback` is spared because the typedef writes it as `(*ak_callback)(`, so the name is
/// followed by `)`, and the types are spared because none of them is ever called.
fn header_declarations(header: &str) -> Vec<String> {
    let text = without_comments(header);
    let bytes = text.as_bytes();
    let mut names = Vec::new();
    let mut from = 0;
    while let Some(at) = text[from..].find("ak_") {
        let start = from + at;
        let mut end = start + 3;
        while end < bytes.len()
            && (bytes[end].is_ascii_lowercase()
                || bytes[end].is_ascii_digit()
                || bytes[end] == b'_')
        {
            end += 1;
        }
        // Only the start of a word, so `armonik_transport_ffi.h` in a path is not a name.
        let whole =
            start == 0 || !(bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_');
        if whole && bytes.get(end) == Some(&b'(') {
            names.push(text[start..end].to_owned());
        }
        from = end;
    }
    names
}
