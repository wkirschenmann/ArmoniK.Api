//! Client streaming through the C ABI: several messages before the half-close.

// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use armonik_transport_ffi::*;
use support::host::*;
use support::COLLECT;

/// What the test sends, and what the server answers having read it all.
const SENT: [&[u8]; 3] = [b"one", b"two", b"three"];

#[test]
fn every_message_of_a_client_stream_crosses_the_abi_before_its_one_reply() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call(channel, COLLECT, &[]);

    for (sent, message) in SENT.iter().enumerate() {
        write_one(host, call, message, sent + 1);
    }

    // Asked before the half-close, because the call is reclaimed the moment its terminal is
    // delivered with nothing owed, and its handle is stale from then on.
    assert_eq!(
        debt_of(call).buffers_lent,
        0,
        "every lent buffer went back with its send"
    );

    assert_eq!(ak_call_end_send(call), ak_status::AK_STATUS_OK);

    let seen = host.recorder.await_terminal();

    assert_eq!(
        seen.kinds()
            .iter()
            .filter(|kind| **kind == ak_event_kind::AK_EVENT_WRITE_DONE)
            .count(),
        SENT.len(),
        "one acquittal per message, and no more"
    );
    assert_eq!(seen.status_code(), Some(0), "{}", seen.status_message());
    assert_eq!(
        seen.message_payloads(),
        vec![b"3:one,two,three".to_vec()],
        "the server read every message, in order, and answered once"
    );

    fixture.close();
}

#[test]
fn a_client_stream_that_sends_nothing_still_reaches_its_reply() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call(channel, COLLECT, &[]);

    assert_eq!(ak_call_end_send(call), ak_status::AK_STATUS_OK);

    let seen = host.recorder.await_terminal();

    assert!(
        !seen.kinds().contains(&ak_event_kind::AK_EVENT_WRITE_DONE),
        "nothing was sent, so nothing is acquitted"
    );
    assert_eq!(seen.status_code(), Some(0), "{}", seen.status_message());
    assert_eq!(seen.message_payloads(), vec![b"0:".to_vec()]);

    fixture.close();
}
