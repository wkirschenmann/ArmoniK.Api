//! How a response reaches the host: in how many callbacks, as `DeliveryCoalescingBytes` gathers
//! what each read of the connection brings.
//!
//! The rounds counted are the process's. Each test holds the one runtime from its first call to
//! its quiescence, so no other test of this binary delivers while it counts.

// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use armonik_transport::hooks;
use armonik_transport_ffi::*;
use support::host::*;
use support::TestServer;

const CALLS: usize = 20;
/// A unary answer whose head, message and trailers the server writes one at a time.
const PACED: &str = "/raw/Paced";

/// How many of `CALLS` paced unary calls in a row, on one channel configured by `json`, reached
/// the host in one callback, past a first call that settles the session's own frames; and how
/// many rounds their deliveries waited.
fn answers_in_one_callback(json: &str) -> (usize, usize) {
    let server = TestServer::start();
    let host = Host::start();
    let channel = host.channel_with(&server.endpoint, json);
    send_one(start_call(channel, PACED, &[]), b"first");
    host.recorder.await_terminals(1);

    let before = hooks::delivery_rounds();
    let whole = (1..=CALLS)
        .filter(|call| {
            send_one(start_call(channel, PACED, &[]), b"hello");
            host.recorder
                .await_terminals(1 + call)
                .last_call_data_callbacks()
                == 1
        })
        .count();
    let rounds = hooks::delivery_rounds() - before;

    ak_channel_release(channel);
    host.stop();
    (whole, rounds)
}

/// A unary answer written in parts reaches the host in one callback: its head, its message and
/// its status, which the connection reads one round of the runtime apart, are gathered rather
/// than delivered as each comes.
#[test]
fn a_unary_answer_reaches_the_host_in_one_callback() {
    let (whole, rounds) = answers_in_one_callback("{}");
    // A call or two that the machine slows past the round its delivery waits may take two.
    assert!(
        whole >= CALLS - 2,
        "{whole} of {CALLS} answers in one callback"
    );
    assert!(rounds > 0, "no delivery waited a round");
}

/// The runtime's channel defaults reach a channel that states nothing, and a channel's own option
/// wins over them.
#[test]
fn a_channel_takes_the_runtime_defaults_under_its_own_options() {
    let rounds = |defaults: &str, json: &str| {
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
    assert_eq!(rounds(r#"{"DeliveryCoalescingBytes":0}"#, "{}"), 0);
    assert!(
        rounds(
            r#"{"DeliveryCoalescingBytes":0}"#,
            r#"{"DeliveryCoalescingBytes":16384}"#
        ) > 0
    );
}

/// With no gathering, a delivery waits no round: each read goes to the host as it comes, and
/// whether an answer's parts come together is the server's pacing alone.
#[test]
fn a_limit_of_zero_waits_no_round() {
    let (_, rounds) = answers_in_one_callback(r#"{"DeliveryCoalescingBytes":0}"#);
    assert_eq!(rounds, 0);
}
