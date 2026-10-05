//! Calls per connection: a channel opens as many connections as its calls in flight need when
//! each carries a limited number, and one for all of them otherwise.

mod common;

use std::time::Duration;

use armonik_transport::grpc::{
    CallControl, CallStartOptions, GrpcChannel, GrpcChannelConfig, GrpcStatusCode, RecvHalf,
    RecvResult, SendHalf,
};
use armonik_transport::http2::TransportConfig;
use bytes::Bytes;
use common::echo::*;
use http::Uri;

fn pooled(endpoint: &str, calls: Option<usize>, idle_timeout: Option<Duration>) -> GrpcChannel {
    let mut transport = TransportConfig::new(Uri::try_from(endpoint).expect("an endpoint"));
    transport.http2.simultaneous_calls_per_connection = calls;
    transport.http2.idle_timeout = idle_timeout;
    channel_with(GrpcChannelConfig::new(transport)).expect("a channel")
}

async fn echo(channel: &GrpcChannel) {
    let (_, messages, status) = unary(
        channel,
        CallStartOptions::new(ECHO),
        Bytes::from_static(b"hello"),
    )
    .await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(messages, vec![Bytes::from_static(b"hello")]);
}

/// A call in flight: its request still open, its head and a first answer in.
async fn chatting(channel: &GrpcChannel) -> (SendHalf, RecvHalf, CallControl) {
    let (mut send, mut recv, control) = channel
        .start_call(CallStartOptions::new(CHAT))
        .expect("the call starts")
        .split();
    send.send_message(Bytes::from_static(b"x"))
        .await
        .expect("the message is accepted");
    recv.recv_head().await.expect("a head");
    assert!(matches!(
        recv.next_message().await,
        Ok(RecvResult::Message(_))
    ));
    (send, recv, control)
}

async fn until(what: &str, ready: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{what}"));
}

#[tokio::test]
async fn by_default_one_connection_carries_every_call() {
    let server = TestServer::start().await;
    let channel = pooled(&server.endpoint, None, None);

    let _calls = [
        chatting(&channel).await,
        chatting(&channel).await,
        chatting(&channel).await,
    ];
    echo(&channel).await;
    assert_eq!(server.connections(), 1);
}

#[tokio::test]
async fn at_one_call_per_connection_each_call_in_flight_has_its_own_and_then_lends_it() {
    let server = TestServer::start().await;
    let channel = pooled(&server.endpoint, Some(1), None);

    let calls = [
        chatting(&channel).await,
        chatting(&channel).await,
        chatting(&channel).await,
    ];
    assert_eq!(server.connections(), 3);

    // Not while they are in flight: a fourth call opens a fourth connection.
    echo(&channel).await;
    assert_eq!(server.connections(), 4);

    drop(calls);
    for _ in 0..5 {
        echo(&channel).await;
    }
    assert_eq!(
        server.connections(),
        4,
        "a call followed another on a free connection"
    );
}

#[tokio::test]
async fn at_two_simultaneous_calls_per_connection_three_calls_take_two() {
    let server = TestServer::start().await;
    let channel = pooled(&server.endpoint, Some(2), None);

    let _calls = [
        chatting(&channel).await,
        chatting(&channel).await,
        chatting(&channel).await,
    ];
    assert_eq!(server.connections(), 2);
}

/// Calls dispatched together, each needing a connection of its own, open as many.
#[tokio::test]
async fn calls_dispatched_together_open_a_connection_each() {
    let server = TestServer::start().await;
    let channel = pooled(&server.endpoint, Some(1), None);

    let calls = futures::future::join_all((0..4).map(|_| chatting(&channel))).await;
    assert_eq!(calls.len(), 4);
    assert_eq!(server.connections(), 4);
}

/// Each connection has its own idle timer: the ones a burst opened close once it is over.
#[tokio::test]
async fn each_connection_closes_on_its_own_idle_timeout() {
    let server = TestServer::start().await;
    let channel = pooled(&server.endpoint, Some(1), Some(Duration::from_millis(200)));

    let calls = [chatting(&channel).await, chatting(&channel).await];
    assert_eq!(server.open(), 2);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(server.open(), 2, "a connection closed under its call");

    drop(calls);
    until("the idle connections closed", || server.open() == 0).await;
    echo(&channel).await;
    assert_eq!(server.connections(), 3, "the next call dialled again");
}

/// A connection closed for idleness leaves the call on another one alone.
#[tokio::test]
async fn closing_an_idle_connection_leaves_another_calls_alone() {
    let server = TestServer::start().await;
    let channel = pooled(&server.endpoint, Some(1), Some(Duration::from_millis(200)));

    let (_send, mut recv, _control) = chatting(&channel).await;
    let (other_send, other_recv, other_control) = chatting(&channel).await;
    assert_eq!(server.connections(), 2);

    // Ended from this side, the second call's connection closes once idle; the first call's
    // stays open under it.
    other_control.cancel();
    drop((other_send, other_recv));
    until("the second connection closed", || server.open() == 1).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(100), recv.next_message())
            .await
            .is_err(),
        "the first call ended with the second's connection"
    );
}

/// A call whose response is whole while its request is still being sent has its request reset,
/// and lends its connection once hyper is done with it.
#[tokio::test]
async fn a_call_whose_response_ends_first_lends_its_connection_once_its_request_is_reset() {
    let server = TestServer::start().await;
    let channel = pooled(&server.endpoint, Some(1), None);

    let (mut send, mut recv, _control) = channel
        .start_call(CallStartOptions::new(ANSWER_EARLY))
        .expect("the call starts")
        .split();
    send.send_message(Bytes::from_static(b"x"))
        .await
        .expect("the message is accepted");
    let (_, _, status) = read_to_terminal(&mut recv).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");

    // Past the reset, which lands microseconds after the terminal; a lease never let go would
    // hold the connection for good.
    tokio::time::sleep(Duration::from_millis(50)).await;
    for _ in 0..3 {
        echo(&channel).await;
    }
    assert_eq!(
        server.connections(),
        1,
        "the connection stayed claimed past the call"
    );
}
