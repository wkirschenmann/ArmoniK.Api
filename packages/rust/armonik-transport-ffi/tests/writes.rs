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

/// What `CALLS` unary calls in a row wrote.
struct Written {
    writes: usize,
    /// How many times a write waited a round of the runtime for more to join it.
    held: usize,
}

/// The writes of `CALLS` unary calls in a row on one channel configured by `json`, each started
/// with `flags`, past a first call that settles the session's own frames.
fn writes_of_unary_calls(json: &str, flags: u32) -> Written {
    let server = TestServer::start();
    let host = Host::start();
    let channel = host.channel_with(&server.endpoint, json);
    let call = |message: &[u8]| {
        let call = start_call_flagged(channel, ECHO, &[], flags);
        if flags & AK_CALL_ONE_REQUEST == 0 {
            send_one(call, message);
        } else {
            send_one_request(call, message);
        }
    };
    call(b"first");
    host.recorder.await_terminals(1);

    let (writes, held) = (hooks::writes(), hooks::held_writes());
    for n in 1..=CALLS {
        call(b"hello");
        host.recorder.await_terminals(1 + n);
    }
    let written = Written {
        writes: hooks::writes() - writes,
        held: hooks::held_writes() - held,
    };

    ak_channel_release(channel);
    host.stop();
    written
}

/// A unary request goes out in one write: its message, handed over from the host's thread while
/// its headers wait, joins them rather than following in a write of its own.
#[test]
fn a_unary_request_goes_out_in_one_write() {
    let writes = writes_of_unary_calls("{}", 0).writes;
    // A call or two that the machine slows past the round its write waits may write twice.
    assert!(writes <= CALLS + 2, "{writes} writes for {CALLS} calls");
}

/// With no gathering, the same request goes out in two writes, its headers and then its message.
#[test]
fn a_limit_of_zero_writes_the_headers_alone() {
    let writes = writes_of_unary_calls(r#"{"Http2":{"Send":{"CoalescingBytes":0}}}"#, 0).writes;
    // The headers go when the connection is first polled after the call starts, and the message
    // comes from another thread after it; a call or two may have it there in time.
    assert!(writes >= 2 * CALLS - 2, "{writes} writes for {CALLS} calls");
}

/// A unary request is held for one round, not two. Its headers are written first and alone,
/// because h2 gives a stream room to send its message only once it opens the stream, on the turn
/// that writes the headers. That write waits a round for the message; the message ends the
/// request, so the write then goes at once.
#[test]
fn a_unary_request_is_held_one_round() {
    for flags in [AK_CALL_ONE_REQUEST, 0] {
        let written = writes_of_unary_calls("{}", flags);
        // A call or two whose end the host sends after that round may be held again.
        assert!(
            written.held <= CALLS + 2,
            "{} rounds held for {CALLS} calls, flags {flags}",
            written.held
        );
        assert!(written.writes <= CALLS + 2, "{} writes", written.writes);
    }
}
