//! The gRPC service the tests use, as a binary, for a host application to call.
//!
//! Takes an optional port; with none, or with zero, it binds an ephemeral one. Either way the chosen
//! address is printed as `listening 127.0.0.1:<port>` on its first line of standard output, which is
//! what a harness reads before it starts calling. It then serves until it is killed.

// The same fixture the Rust tests use, so the two sides cannot drift apart.
#[path = "../tests/common/mod.rs"]
mod common;

use std::io::Write;

use bytes::Bytes;

fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .map(|argument| {
            argument
                .parse()
                .expect("the first argument, when given, is a port number")
        })
        .unwrap_or(0);

    // Echoing as each message arrives is the one behaviour that serves every call shape: a caller
    // that sends one message and closes gets a unary call, and one that keeps sending gets a
    // bidirectional one, without the service having to be told which.
    let address =
        common::server::serve_at(common::server::TestService::echo_each(Bytes::new()), port);
    println!("listening {address}");
    // A harness blocks on that line, and a pipe that is never flushed would leave it blocked.
    std::io::stdout().flush().expect("flush the address");

    // `serve_at` runs the server on a runtime of its own, so this thread has nothing left to do but
    // stay alive.
    std::thread::park();
}
