//! How a response reaches the host: in how many callbacks, as `Grpc.Host.Receive.CoalescingBytes` gathers
//! what each read of the connection brings.
//!
//! The rounds counted are the process's. Each test holds the one runtime from its first call to
//! its quiescence, so no other test of this binary delivers while it counts.

// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use std::sync::Arc;
use std::time::Duration;

use armonik_transport::hooks;
use armonik_transport_ffi::*;
use support::host::*;
use support::{release_paced_part, TestServer};

const CALLS: usize = 20;
/// A unary answer whose server writes its head at once, and its message and its trailers each
/// when `release_paced_part` lets it.
const PACED: &str = "/raw/Paced";
/// The parts of a paced answer past its head: a read waits for each.
const PARTS: usize = 2;
/// Long enough for a part the server is let to write to reach the client's socket, far past what
/// a write on the loopback takes.
const IN_FLIGHT: Duration = Duration::from_millis(10);

/// Paces `PACED` by the client's reads: a read that finds nothing lets the server write the next
/// part, which is at the client's socket before the delivery's round reads the connection.
fn pace_by_reads() {
    hooks::when_read_waits(Some(Arc::new(|| {
        release_paced_part();
        std::thread::sleep(IN_FLIGHT);
    })));
}

/// What `CALLS` paced unary calls in a row did, on one channel configured by `json`, past a
/// first call that settles the session's own frames.
struct Paced {
    /// The rounds their deliveries waited.
    rounds: usize,
    /// The calls each of whose rounds found the part it waited for.
    answered: usize,
    /// Those of `answered` that reached the host in one callback.
    answered_whole: usize,
}

fn paced_calls(json: &str) -> Paced {
    pace_by_reads();
    let server = TestServer::start();
    let host = Host::start();
    let channel = host.channel_with(&server.endpoint, json);
    send_one(start_call(channel, PACED, &[]), b"first");
    host.recorder.await_terminals(1);

    let before = hooks::delivery_rounds();
    let (mut answered, mut answered_whole) = (0, 0);
    for call in 1..=CALLS {
        let found = hooks::delivery_rounds_answered();
        send_one(start_call(channel, PACED, &[]), b"hello");
        let callbacks = host
            .recorder
            .await_terminals(1 + call)
            .last_call_data_callbacks();
        if hooks::delivery_rounds_answered() - found == PARTS {
            answered += 1;
            answered_whole += usize::from(callbacks == 1);
        }
    }
    let rounds = hooks::delivery_rounds() - before;

    ak_channel_release(channel);
    host.stop();
    Paced {
        rounds,
        answered,
        answered_whole,
    }
}

/// A unary answer written in parts reaches the host in one callback: its head, its message and
/// its status, which the connection reads one round of the runtime apart, are gathered rather
/// than delivered as each comes.
#[test]
fn a_unary_answer_reaches_the_host_in_one_callback() {
    let paced = paced_calls("{}");
    assert_eq!(
        paced.rounds,
        PARTS * CALLS,
        "a delivery waits a round for each part"
    );
    // A round ends before the runtime reads the connection when something else wakes the call's
    // task, so only the calls whose rounds all found their part show the gathering. Every call
    // missing is a round that no longer reads the connection.
    assert!(paced.answered > 0, "no call's rounds found their parts");
    assert_eq!(
        paced.answered_whole, paced.answered,
        "answers whose rounds found their parts, in one callback"
    );
}

/// The runtime's channel defaults reach a channel that states nothing, and a channel's own option
/// wins over them.
#[test]
fn a_channel_takes_the_runtime_defaults_under_its_own_options() {
    let rounds = |defaults: &str, json: &str| {
        pace_by_reads();
        let server = TestServer::start();
        let host = Host::with_channel_defaults(defaults);
        let channel = host.channel_with(&server.endpoint, json);
        let before = hooks::delivery_rounds();
        for call in 1..=CALLS {
            send_one(start_call(channel, PACED, &[]), b"hello");
            host.recorder.await_terminals(call);
        }
        let rounds = hooks::delivery_rounds() - before;
        ak_channel_release(channel);
        host.stop();
        rounds
    };
    assert_eq!(
        rounds(
            r#"{"Grpc":{"Host":{"Receive":{"CoalescingBytes":0}}}}"#,
            "{}"
        ),
        0
    );
    assert_eq!(
        rounds(
            r#"{"Grpc":{"Host":{"Receive":{"CoalescingBytes":0}}}}"#,
            r#"{"Grpc":{"Host":{"Receive":{"CoalescingBytes":16384}}}}"#
        ),
        PARTS * CALLS
    );
}

/// With no gathering, a delivery waits no round: each read goes to the host as it comes.
#[test]
fn a_limit_of_zero_waits_no_round() {
    let paced = paced_calls(r#"{"Grpc":{"Host":{"Receive":{"CoalescingBytes":0}}}}"#);
    assert_eq!(paced.rounds, 0);
}
