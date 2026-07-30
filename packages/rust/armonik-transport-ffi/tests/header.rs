//! Checks on the generated C header, which is a committed artefact rather than a build output.
//!
//! `build.rs` regenerates `include/armonik_transport_ffi.h` on every build, so these run against
//! whatever the current sources produce.

const HEADER: &str = include_str!("../include/armonik_transport_ffi.h");

/// Names that legitimately appear as `#define` without the `AK_` prefix.
const ALLOWED_UNPREFIXED: &[&str] = &["ARMONIK_TRANSPORT_FFI_H"];

#[test]
fn every_macro_is_namespaced() {
    // A C header has no modules. `#define OK 0` — which is what the Rust `status::OK` produces
    // unrenamed — would collide with a great deal of existing code, so every macro this header
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
        "ak_call_start",
        "ak_call_wait_handle",
        "ak_call_try_recv",
        "ak_call_try_headers",
        "ak_call_try_send",
        "ak_call_close_send",
        "ak_call_status",
        "ak_call_cancel",
        "ak_call_free",
        "ak_bytes_free",
        "ak_log_init",
        "ak_log_drain",
    ] {
        assert!(
            HEADER.contains(symbol),
            "`{symbol}` is missing from the generated header"
        );
    }
}

#[test]
fn the_ownership_rules_are_spelled_out() {
    // These are the rules a caller cannot infer from the signatures, and getting any of them wrong
    // is a memory bug rather than a compile error. If the preamble in `cbindgen.toml` is ever
    // trimmed, this is the reminder that the contract went with it.
    for phrase in [
        "ak_bytes_free",
        "never be dereferenced",
        "never freed by this library",
        "NATIVE byte order",
        "BORROWED",
        "Never close it",
    ] {
        assert!(
            HEADER.contains(phrase),
            "the header no longer documents {phrase:?}"
        );
    }
}
