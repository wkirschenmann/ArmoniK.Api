//! Renders armonik-transport-ffi's C header and the P/Invoke half of the .NET binding from the
//! crate's Rust declarations.
//!
//! Both are committed, so a reader sees the ABI without running anything. Without arguments this
//! writes them; with `--check` it compares instead and fails on a difference, which is what the
//! .NET build runs.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The files the ABI is declared in. Every exported item lives in one of the two.
const SOURCES: [&str; 2] = ["src/abi.rs", "src/lib.rs"];

const HEADER: &str = "packages/rust/armonik-transport-ffi/include/armonik_transport_ffi.h";
const NATIVE_METHODS: &str =
    "packages/csharp/ArmoniK.Api.Client.RustGrpcChannel/Interop/NativeMethods.g.cs";

const CSHARP_HEADER: &str = "\
// This file is part of the ArmoniK project
//
// Copyright (C) ANEO, 2021-2026. All rights reserved.
//
// Licensed under the Apache License, Version 2.0 (the \"License\")
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an \"AS IS\" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

// Rendered by csbindgen from armonik-transport-ffi's src/abi.rs and src/lib.rs. Edits here are
// lost: change the Rust, then run `cargo run -p armonik-transport-ffi-bindgen`. The build fails
// while the two differ.
";

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

/// csbindgen writes only to a file, so it writes to one of its own and this reads it back.
fn native_methods() -> String {
    let mut builder = csbindgen::Builder::default();
    for source in SOURCES {
        builder = builder.input_extern_file(ffi().join(source));
    }
    let scratch = std::env::temp_dir().join(format!(
        "armonik-transport-ffi-bindgen-{}.g.cs",
        std::process::id()
    ));
    builder
        // Named by no signature: the first read from the event's status code, the second written
        // into a source's kind.
        .always_included_types(["ak_head_origin", "ak_source_kind"])
        // A delegate, which the marshaller turns into a pointer on every framework the binding
        // targets. A C# function pointer's target has to be UnmanagedCallersOnly, which
        // netstandard2.0 does not have.
        .csharp_use_function_pointer(false)
        .csharp_generate_const_filter(|name| name.starts_with("AK_"))
        // NativeMethods.cs names the library, beside the loader that finds it.
        .csharp_disable_emit_dll_name(true)
        .csharp_namespace("ArmoniK.Api.Client.RustGrpcChannel.Interop")
        .csharp_class_name("NativeMethods")
        .csharp_file_header(CSHARP_HEADER)
        .generate_csharp_file(&scratch)
        .expect("the ABI renders as C#");
    let rendered = std::fs::read_to_string(&scratch).expect("csbindgen wrote what it rendered");
    let _ = std::fs::remove_file(&scratch);
    rendered
}

/// Every committed file this renders, beside what it renders, with LF line endings.
fn rendered() -> [(&'static str, String); 2] {
    [(HEADER, header()), (NATIVE_METHODS, native_methods())]
        .map(|(path, text)| (path, text.replace("\r\n", "\n")))
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
