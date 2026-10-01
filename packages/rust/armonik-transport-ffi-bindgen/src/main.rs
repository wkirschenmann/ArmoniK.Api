//! Renders armonik-transport-ffi's C header from the crate's Rust declarations.
//!
//! The header is committed, so a reader sees the ABI without running anything. Without arguments
//! this writes it; with `--check` it compares instead and fails on a difference.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The files the ABI is declared in. Every exported item lives in one of the two.
const SOURCES: [&str; 2] = ["src/abi.rs", "src/lib.rs"];

const HEADER: &str = "packages/rust/armonik-transport-ffi/include/armonik_transport_ffi.h";

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn ffi() -> PathBuf {
    repository().join("packages/rust/armonik-transport-ffi")
}

fn header() -> String {
    let config = cbindgen::Config::from_file(ffi().join("cbindgen.toml"))
        .expect("cbindgen.toml is committed beside the crate");
    let mut builder = cbindgen::Builder::new().with_config(config);
    for source in SOURCES {
        builder = builder.with_src(ffi().join(source));
    }
    let mut rendered = Vec::new();
    builder
        .generate()
        .expect("the ABI renders as C")
        .write(&mut rendered);
    String::from_utf8(rendered).expect("cbindgen writes UTF-8")
}

/// Every committed file this renders, beside what it renders, with LF line endings.
fn rendered() -> [(&'static str, String); 1] {
    [(HEADER, header())].map(|(path, text)| (path, text.replace("\r\n", "\n")))
}

/// Line endings aside, which a checkout may change.
fn read(path: &str) -> Option<String> {
    std::fs::read_to_string(repository().join(path))
        .ok()
        .map(|text| text.replace("\r\n", "\n"))
}

/// The committed files that are not what the Rust renders.
fn stale() -> Vec<&'static str> {
    rendered()
        .into_iter()
        .filter(|(path, text)| read(path).as_deref() != Some(text.as_str()))
        .map(|(path, _)| path)
        .collect()
}

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        None => {
            for (path, text) in rendered() {
                std::fs::write(repository().join(path), text)
                    .unwrap_or_else(|error| panic!("{path} cannot be written: {error}"));
                println!("wrote {path}");
            }
            ExitCode::SUCCESS
        }
        Some("--check") => {
            let stale = stale();
            for path in &stale {
                eprintln!(
                    "{path} is not what armonik-transport-ffi declares; write it again with \
                     `cargo run -p armonik-transport-ffi-bindgen`"
                );
            }
            if stale.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Some(other) => {
            eprintln!("unknown argument {other}; the only one is --check");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_committed_bindings_are_the_ones_the_rust_declares() {
        assert_eq!(
            super::stale(),
            Vec::<&str>::new(),
            "write them again with `cargo run -p armonik-transport-ffi-bindgen`"
        );
    }
}
