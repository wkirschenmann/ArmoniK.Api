//! Writes the JSON schema of the channel options, which is committed beside this crate.
//!
//! The schema is a file rather than a build artefact because the generator that turns it into a
//! C# class runs at design time too, and a schema that appeared only after a build would leave a
//! fresh clone with no type. `the_committed_schema_is_the_one_the_types_describe` is what keeps
//! the file honest.
//!
//! The path is an argument rather than a shell redirection: `>` is the shell's, and PowerShell's
//! writes UTF-16 or a byte order mark depending on its version, neither of which is what a test
//! comparing bytes expects.

use std::io::Write;

fn main() {
    let rendered = armonik_transport::options::schema();

    match std::env::args().nth(1) {
        Some(path) => std::fs::write(&path, rendered)
            .unwrap_or_else(|failure| panic!("`{path}` cannot be written: {failure}")),
        None => std::io::stdout()
            .write_all(rendered.as_bytes())
            .expect("stdout takes the schema"),
    }
}
