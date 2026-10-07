//! Writes the JSON schema of the runtime options, which is committed beside this crate.
//!
//! The path is an argument rather than a shell redirection, for the reason the channel schema's
//! example gives: PowerShell's `>` writes UTF-16 or a byte order mark.

use std::io::Write;

fn main() {
    let rendered = armonik_transport::options::runtime_schema();

    match std::env::args().nth(1) {
        Some(path) => std::fs::write(&path, rendered)
            .unwrap_or_else(|failure| panic!("`{path}` cannot be written: {failure}")),
        None => std::io::stdout()
            .write_all(rendered.as_bytes())
            .expect("stdout takes the schema"),
    }
}
