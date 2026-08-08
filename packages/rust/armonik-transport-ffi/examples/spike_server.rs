//! The raw gRPC service the integration tests use, as a binary, for the C# harness to call.
//!
//! Takes an optional port; with none, or with zero, it binds an ephemeral one. Either way the
//! chosen address is printed as `listening 127.0.0.1:<port>` on its first line of standard output,
//! which is what the harness reads before it starts calling. It then serves until it is killed.

// The same fixture the Rust tests use, so the two sides of the checklist cannot drift apart.
#[path = "../tests/common/mod.rs"]
mod common;

use std::io::Write;

fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .map(|argument| {
            argument
                .parse()
                .expect("the first argument, when given, is a port number")
        })
        .unwrap_or(0);

    let address = common::spike::serve_spike(port);
    println!("listening {address}");
    // The harness blocks on that line, and a pipe that is never flushed would leave it blocked.
    std::io::stdout().flush().expect("flush the address");

    // `serve_spike` runs the server on a runtime of its own, so this thread has nothing left to do
    // but stay alive.
    std::thread::park();
}
