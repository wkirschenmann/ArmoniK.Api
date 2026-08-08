//! Generates the C header for this crate's ABI.
//!
//! The header is written into the source tree, at `include/armonik_transport_ffi.h`, and committed:
//! it is the artefact a reviewer reads to see the whole contract at once, and the file a caller's own
//! declarations are checked against. Generating it into `OUT_DIR` instead would put it somewhere
//! nobody looks.
//!
//! Because it is committed, a build that changes the ABI also changes a tracked file - which is the
//! point: an ABI change should show up in the diff rather than only in a `.dll`.

use std::path::PathBuf;

fn main() {
    let crate_dir = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("cargo always sets CARGO_MANIFEST_DIR"),
    );
    let header = crate_dir.join("include").join("armonik_transport_ffi.h");

    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=cbindgen.toml");
    println!("cargo:rerun-if-changed=build.rs");

    let config = cbindgen::Config::from_file(crate_dir.join("cbindgen.toml"))
        .expect("cbindgen.toml should be readable and valid");

    match cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()
    {
        Ok(bindings) => {
            // `write_to_file` leaves the file alone when the content is unchanged, so an ordinary
            // rebuild does not keep touching a tracked file's mtime.
            bindings.write_to_file(&header);
        }
        Err(error) => {
            // A failure here must not break `cargo build`: the header is a review and binding aid,
            // not an input to compiling this crate, and the most common cause is a toolchain that
            // cannot expand macros in a dependency. Warn loudly instead.
            println!(
                "cargo:warning=could not generate {}: {error}",
                header.display()
            );
        }
    }
}
