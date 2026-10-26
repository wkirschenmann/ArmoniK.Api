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

/// What the calls of a test wrote.
struct Written {
    writes: usize,
    /// How many times a write waited a round of the runtime for more to join it.
    held: usize,
}

/// The writes of `CALLS` unary calls in a row on one channel configured by `json`, past a first
/// call that settles the session's own frames.
fn writes_of_unary_calls(json: &str) -> Written {
    let server = TestServer::start();
    let host = Host::start();
    let channel = host.channel_with(&server.endpoint, json);
    send_one(start_call(channel, ECHO, &[]), b"first");
    host.recorder.await_terminals(1);

    let (writes, held) = (hooks::writes(), hooks::held_writes());
    for call in 1..=CALLS {
        send_one(start_call(channel, ECHO, &[]), b"hello");
        host.recorder.await_terminals(1 + call);
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
    let written = writes_of_unary_calls("{}");
    // A call or two that the machine slows past the round its write waits may write twice.
    assert!(
        written.writes <= CALLS + 2,
        "{} writes for {CALLS} calls",
        written.writes
    );
    // The headers are written alone first: h2 gives a stream room to send its message only once
    // it opens the stream, on the turn that writes the headers. That write waits, one round: the
    // message joins it during the round.
    assert!(
        (CALLS..=CALLS + 2).contains(&written.held),
        "{} rounds held for {CALLS} calls",
        written.held
    );
}

/// With no gathering, the same request goes out in two writes, its headers and then its message.
#[test]
fn a_limit_of_zero_writes_the_headers_alone() {
    let written = writes_of_unary_calls(r#"{"Http2":{"Send":{"CoalescingBytes":0}}}"#);
    // The headers go when the connection is first polled after the call starts, and the message
    // comes from another thread after it; a call or two may have it there in time.
    assert!(
        written.writes >= 2 * CALLS - 2,
        "{} writes for {CALLS} calls",
        written.writes
    );
    assert_eq!(written.held, 0);
}

/// What `BURSTS` bursts of `burst` one-request unary calls wrote, started and sent from one host
/// thread on one channel: the writes, and the rounds they waited.
fn writes_of_bursts(burst: usize) -> Written {
    let server = TestServer::start();
    let host = Host::start();
    let channel = host.channel(&server.endpoint);
    send_one_request(
        start_call_flagged(channel, ECHO, &[], AK_CALL_ONE_REQUEST),
        b"first",
    );
    host.recorder.await_terminals(1);

    let (writes, held) = (hooks::writes(), hooks::held_writes());
    for round in 0..BURSTS {
        let calls: Vec<ak_handle> = (0..burst)
            .map(|_| start_call_flagged(channel, ECHO, &[], AK_CALL_ONE_REQUEST))
            .collect();
        for call in calls {
            send_one_request(call, b"hello");
        }
        host.recorder.await_terminals(1 + (round + 1) * burst);
    }
    let written = Written {
        writes: hooks::writes() - writes,
        held: hooks::held_writes() - held,
    };

    ak_channel_release(channel);
    host.stop();
    written
}

const BURSTS: usize = 10;

/// Calls in flight together share their writes: a write waits while a round adds another call's
/// frames to it, not only its own call's, so a burst goes out in a few writes rather than one or
/// more per call.
#[test]
fn calls_in_flight_together_share_their_writes() {
    const BURST: usize = 8;
    let written = writes_of_bursts(BURST);
    // A few writes per burst: a request whose end is written still waits, for the next calls.
    assert!(
        written.writes <= BURST * BURSTS / 2,
        "{} writes for {} calls",
        written.writes,
        BURST * BURSTS
    );
    // A write waits whole rounds, not a round each time a call of the burst hands the connection
    // a frame: the connection takes in what the round brought in one poll at its end.
    assert!(
        written.held <= 2 * written.writes,
        "{} rounds held for {} writes",
        written.held,
        written.writes
    );
}

/// A burst longer than the runtime runs between two reads of its events still gathers: the
/// runtime ends the round with calls still ready, and what they hand the connection after it
/// earns the write another round.
#[test]
fn a_long_burst_gathers_past_the_runtime_reading_its_events() {
    const BURST: usize = 64;
    let written = writes_of_bursts(BURST);
    assert!(
        written.writes <= BURST * BURSTS / 8,
        "{} writes for {} calls",
        written.writes,
        BURST * BURSTS
    );
}
