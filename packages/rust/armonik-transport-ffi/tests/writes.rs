//! What the library writes to the network, counted with the transport's test hooks.
//!
//! The count is the process's. Each test holds the one runtime from its first write to its
//! quiescence, so no other test of this binary writes while it counts.

// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use armonik_transport::hooks;
use armonik_transport_ffi::*;
use support::host::*;
use support::{TestServer, ECHO};

const CALLS: usize = 20;

/// The writes of `CALLS` unary calls in a row on one channel configured by `json`, past a first
/// call that settles the session's own frames.
fn writes_of_unary_calls(json: &str) -> usize {
    let server = TestServer::start();
    let host = Host::start();
    let channel = host.channel_with(&server.endpoint, json);
    send_one(start_call(channel, ECHO, &[]), b"first");
    host.recorder.await_terminals(1);

    let before = hooks::writes();
    for call in 1..=CALLS {
        send_one(start_call(channel, ECHO, &[]), b"hello");
        host.recorder.await_terminals(1 + call);
    }
    let writes = hooks::writes() - before;

    ak_channel_release(channel);
    host.stop();
    writes
}

/// A unary request goes out in one write: its message, handed over from the host's thread while
/// its headers wait, joins them rather than following in a write of its own.
#[test]
fn a_unary_request_goes_out_in_one_write() {
    let writes = writes_of_unary_calls("{}");
    // A call or two that the machine slows past the round its write waits may write twice.
    assert!(writes <= CALLS + 2, "{writes} writes for {CALLS} calls");
}

/// With no gathering, the same request goes out in two writes, its headers and then its message.
#[test]
fn a_limit_of_zero_writes_the_headers_alone() {
    let writes = writes_of_unary_calls(r#"{"Http2":{"Send":{"CoalescingBytes":0}}}"#);
    // The headers go when the connection is first polled after the call starts, and the message
    // comes from another thread after it; a call or two may have it there in time.
    assert!(writes >= 2 * CALLS - 2, "{writes} writes for {CALLS} calls");
}
