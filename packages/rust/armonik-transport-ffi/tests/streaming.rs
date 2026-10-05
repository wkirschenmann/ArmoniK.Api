//! Client streaming through the C ABI: several messages before the half-close.

// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use armonik_transport_ffi::*;
use support::host::*;
use support::{CHAT, COLLECT, FAN};

/// What the test sends, and what the server answers having read it all.
const SENT: [&[u8]; 3] = [b"one", b"two", b"three"];

/// One more, accepted and then cancelled under, so it never reaches the server.
const ABANDONED: &[u8] = b"four";

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

    assert_eq!(
        unsafe { ak_call_end_send(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );

    let seen = host.recorder.await_terminal();

    let kinds = seen.kinds();
    assert_eq!(
        acquittals(&kinds).len(),
        SENT.len(),
        "one acquittal per message, and no more: {kinds:?}"
    );
    assert!(
        acquittals(&kinds).iter().all(|at| *at < terminal(&kinds)),
        "every acquittal is in before the terminal: {kinds:?}"
    );
    let reply = kinds
        .iter()
        .position(|kind| *kind == ak_event_kind::AK_EVENT_MESSAGE)
        .expect("the server answered");
    assert!(
        acquittals(&kinds).iter().all(|at| *at < reply),
        "and this server answers having read them all, so all three precede its reply: {kinds:?}"
    );
    assert_eq!(seen.status_code(), Some(0), "{}", seen.status_message());
    assert_eq!(
        seen.message_payloads(),
        vec![b"3:one,two,three".to_vec()],
        "the server read every message, in order, and answered once"
    );

    fixture.close();
}

/// Where the acquittal each side counts on is owed for a message that never reached the wire.
///
/// The header settles it: WRITE_DONE settles an accepted send and says nothing about the network -
/// the message may have been written, or abandoned because the call was cancelled - and it arrives
/// exactly once per accepted send, always before the terminal. So a send accepted and then
/// cancelled under is acquitted like one written, and a host waiting on its buffer is not left
/// waiting for a call that is already over.
///
/// The interleaving this reaches depends on whether the writer dequeues before the cancel lands.
/// Both are the same assertion, which is the point: the count and the order hold either way.
#[test]
fn a_send_the_transport_abandoned_is_acquitted_like_one_it_wrote() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call(channel, COLLECT, &[]);

    for (sent, message) in SENT.iter().enumerate() {
        write_one(host, call, message, sent + 1);
    }

    // Accepted, and the call ended under it: `SendHalf::send_message` answers `Ended` once the
    // call's `over` is set, and the queued command is drained rather than dropped so its
    // acquittal still goes out.
    let (status, buffer) = lend(call, ABANDONED.len());
    assert_eq!(status, ak_status::AK_STATUS_OK);
    unsafe { std::ptr::copy_nonoverlapping(ABANDONED.as_ptr(), buffer.ptr, ABANDONED.len()) };
    assert_eq!(
        unsafe { ak_call_send_message(call, buffer, buffer.len, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK,
        "the send is accepted while the call is live"
    );
    assert_eq!(
        unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );

    let seen = host.recorder.await_terminal();
    let kinds = seen.kinds();

    assert_eq!(
        acquittals(&kinds).len(),
        SENT.len() + 1,
        "the abandoned send is acquitted too: {kinds:?}"
    );
    assert!(
        acquittals(&kinds).iter().all(|at| *at < terminal(&kinds)),
        "and before the terminal, like the others: {kinds:?}"
    );

    fixture.close();
}

/// A message past `Grpc.Send.MaxMessageSize` is accepted at the ABI and acquitted like any other,
/// and the call ends RESOURCE_EXHAUSTED: the refusal is the transport's, and the terminal reports
/// it.
#[test]
fn a_message_past_the_send_limit_ends_the_call_resource_exhausted() {
    const RESOURCE_EXHAUSTED: i32 = 8;

    let server = support::TestServer::start();
    let host = Host::start();
    let channel = host.channel_with(
        &server.endpoint,
        r#"{"Grpc":{"Send":{"MaxMessageSize":3}}}"#,
    );
    let call = start_call(channel, COLLECT, &[]);

    write_one(&host, call, b"one", 1);
    let (status, buffer) = lend(call, ABANDONED.len());
    assert_eq!(status, ak_status::AK_STATUS_OK);
    unsafe { std::ptr::copy_nonoverlapping(ABANDONED.as_ptr(), buffer.ptr, ABANDONED.len()) };
    assert_eq!(
        unsafe { ak_call_send_message(call, buffer, buffer.len, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK,
        "the ABI accepts it: the limit is the transport's"
    );

    let seen = host.recorder.await_terminal();
    let kinds = seen.kinds();
    assert_eq!(
        acquittals(&kinds).len(),
        2,
        "the refused send is acquitted too: {kinds:?}"
    );
    assert!(
        acquittals(&kinds).iter().all(|at| *at < terminal(&kinds)),
        "and before the terminal: {kinds:?}"
    );
    assert_eq!(
        seen.status_code(),
        Some(RESOURCE_EXHAUSTED),
        "{}",
        seen.status_message()
    );
    assert!(seen.message_payloads().is_empty());

    ak_channel_release(channel);
    host.stop();
}

/// Where each acquittal sits in the sequence the host was handed.
fn acquittals(kinds: &[ak_event_kind]) -> Vec<usize> {
    kinds
        .iter()
        .enumerate()
        .filter(|(_, kind)| **kind == ak_event_kind::AK_EVENT_WRITE_DONE)
        .map(|(at, _)| at)
        .collect()
}

fn terminal(kinds: &[ak_event_kind]) -> usize {
    kinds
        .iter()
        .position(|kind| *kind == ak_event_kind::AK_EVENT_STATUS)
        .expect("the terminal was awaited")
}

/// Sends and answers interleaved across the ABI, one message each way at a time.
#[test]
fn a_bidi_call_crosses_the_abi_with_sends_and_answers_interleaved() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call(channel, CHAT, &[]);

    for (sent, message) in SENT.iter().enumerate() {
        write_one(host, call, message, sent + 1);

        // The echo answers a message only after receiving it, so the answer to this message is
        // what arrives next.
        let seen = host.recorder.await_messages(sent + 1);
        assert_eq!(seen.message_payloads()[sent], message.to_vec());
    }

    assert_eq!(
        unsafe { ak_call_end_send(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );

    let seen = host.recorder.await_terminal();
    assert_eq!(seen.status_code(), Some(0), "{}", seen.status_message());

    fixture.close();
}

#[test]
fn every_message_of_a_server_stream_crosses_the_abi_in_order() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call(channel, FAN, &[]);

    send_one(call, b"one,two,three");

    let seen = host.recorder.await_terminal();

    assert_eq!(seen.status_code(), Some(0), "{}", seen.status_message());
    assert_eq!(
        seen.message_payloads(),
        vec![b"one".to_vec(), b"two".to_vec(), b"three".to_vec()],
        "one event per message, in order"
    );

    fixture.close();
}

#[test]
fn a_client_stream_that_sends_nothing_still_reaches_its_reply() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call(channel, COLLECT, &[]);

    assert_eq!(
        unsafe { ak_call_end_send(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );

    let seen = host.recorder.await_terminal();

    assert!(
        !seen.kinds().contains(&ak_event_kind::AK_EVENT_WRITE_DONE),
        "nothing was sent, so nothing is acquitted"
    );
    assert_eq!(seen.status_code(), Some(0), "{}", seen.status_message());
    assert_eq!(seen.message_payloads(), vec![b"0:".to_vec()]);

    fixture.close();
}
