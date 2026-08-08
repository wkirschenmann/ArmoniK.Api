//! Checks on the generated C header, which is a committed artefact rather than a build output.
//!
//! `build.rs` regenerates `include/armonik_transport_ffi.h` on every build, so these run against
//! whatever the current sources produce.

const HEADER: &str = include_str!("../include/armonik_transport_ffi.h");

/// Names that legitimately appear as `#define` without the `AK_` prefix.
const ALLOWED_UNPREFIXED: &[&str] = &["ARMONIK_TRANSPORT_FFI_H"];

#[test]
fn every_macro_is_namespaced() {
    // A C header has no modules. `#define OK 0` - which is what the Rust `status::OK` produces
    // unrenamed - would collide with a great deal of existing code, so every macro this header
    // defines has to carry the `AK_` prefix. The rename table in `cbindgen.toml` does that; this
    // test is what stops a newly-added constant from quietly skipping it.
    let offenders: Vec<&str> = HEADER
        .lines()
        .filter_map(|line| line.strip_prefix("#define "))
        .filter_map(|rest| rest.split_whitespace().next())
        // Function-like macros would carry their parameter list; none exist today, but split on `(`
        // so one appearing later is still checked by name.
        .map(|name| name.split('(').next().unwrap_or(name))
        .filter(|name| !name.starts_with("AK_"))
        .filter(|name| !ALLOWED_UNPREFIXED.contains(name))
        .collect();

    assert!(
        offenders.is_empty(),
        "these macros need an `AK_` prefix (add them to `[export.rename]` in cbindgen.toml): {offenders:?}"
    );
}

#[test]
fn every_entry_point_is_declared() {
    // The header is what a caller's own declarations are written against, so an entry point missing
    // from it is a function no caller can reach. cbindgen only emits `#[no_mangle]`
    // `pub extern "C"` items, so this catches one losing its attribute as much as a generation
    // failure.
    for symbol in [
        "ak_client_create",
        "ak_client_free",
        "ak_request_start",
        "ak_request_write",
        "ak_request_close_send",
        "ak_request_read",
        "ak_request_cancel",
        "ak_request_free",
        "ak_request_on_event",
        "ak_bytes_free",
    ] {
        assert!(
            HEADER.contains(symbol),
            "`{symbol}` is missing from the generated header"
        );
    }
}

#[test]
fn every_status_and_event_is_declared() {
    // A caller compares against these by name. One that never reaches the header is a code the
    // other side has to hard-wire as a number.
    for constant in [
        "AK_OK",
        "AK_NULL_ARGUMENT",
        "AK_INVALID_UTF8",
        "AK_INVALID_CONFIG",
        "AK_CONNECTION_FAILED",
        "AK_INVALID_HANDLE",
        "AK_INVALID_STATE",
        "AK_INTERNAL",
        "AK_INTERNAL_PANIC",
        "AK_CANCELLED",
        "AK_TIMEOUT",
        "AK_TRANSPORT",
        "AK_EVENT_RESPONSE_HEADERS",
        "AK_EVENT_WRITE_DONE",
        "AK_EVENT_READ_DONE",
        "AK_EVENT_COMPLETED",
    ] {
        assert!(
            HEADER.contains(&format!("#define {constant} ")),
            "`{constant}` is missing from the generated header"
        );
    }
}

#[test]
fn the_contract_the_signatures_cannot_carry_is_spelled_out() {
    // These are the rules a caller cannot infer from the signatures, and getting any of them wrong
    // is a memory bug or a hang rather than a compile error. If the preamble in `cbindgen.toml` is
    // ever trimmed, this is the reminder that the contract went with it.
    for phrase in [
        // Ownership.
        "ak_bytes_free",
        "never dereference it",
        "BORROWED for the duration of the callback",
        "NATIVE byte order",
        // Thread affinity, which a caller has to know before it hands a handle to a thread pool.
        "Handles are thread-safe",
        "synchronous with respect to the event callback",
        // The reactor's three rules.
        "Nothing arrives unarmed",
        "exactly once, and last",
        "No callback happens during an inbound call",
        // What a callback may not do.
        "must not block",
        "must not raise",
        "must not re-enter",
    ] {
        assert!(
            HEADER.contains(phrase),
            "the header no longer documents {phrase:?}"
        );
    }
}
