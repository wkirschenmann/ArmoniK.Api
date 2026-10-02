//! An idle session is closed after the channel's idle timeout, and the next call dials again.

mod common;

use std::time::Duration;

use armonik_transport::grpc::{
    CallStartOptions, Deadline, GrpcChannel, GrpcChannelConfig, GrpcStatusCode,
};
use armonik_transport::http2::{TlsConfig, TransportConfig};
use bytes::Bytes;
use common::echo::*;
use common::tls::{Pki, TlsServer};
use http::Uri;

fn idling_after(endpoint: &str, idle_timeout: Option<Duration>) -> GrpcChannel {
    let mut transport = TransportConfig::new(Uri::try_from(endpoint).expect("an endpoint"));
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
async fn an_idle_session_is_closed_and_the_next_call_dials_again() {
    let server = TestServer::start().await;
    let channel = idling_after(&server.endpoint, Some(Duration::from_millis(200)));

    echo(&channel).await;
    until("the idle session closed", || server.open() == 0).await;
    assert_eq!(server.connections(), 1);

    echo(&channel).await;
    assert_eq!(server.connections(), 2, "the call dialled a new session");
}

#[tokio::test]
async fn with_no_idle_timeout_the_session_stays_open() {
    let server = TestServer::start().await;
    let channel = idling_after(&server.endpoint, None);

    echo(&channel).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(server.open(), 1);
    echo(&channel).await;
    assert_eq!(server.connections(), 1);
}

/// A call holds the session to the end of its response, however long past the idle timeout:
/// this one has its head and a message, and its stream stays open.
#[tokio::test]
async fn a_call_under_way_keeps_the_session_open() {
    let server = TestServer::start().await;
    let channel = idling_after(&server.endpoint, Some(Duration::from_millis(200)));

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
        Ok(armonik_transport::grpc::RecvResult::Message(_))
    ));

    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(server.open(), 1, "the session closed under a call");
    echo(&channel).await;
    assert_eq!(server.connections(), 1, "a second session was dialled");

    control.cancel();
    until("the idle session closed after the call", || {
        server.open() == 0
    })
    .await;
}

/// A call inside the timeout starts it again: the session lasts the timeout after the last call.
#[tokio::test]
async fn a_call_within_the_timeout_starts_it_again() {
    let server = TestServer::start().await;
    let channel = idling_after(&server.endpoint, Some(Duration::from_millis(1000)));

    echo(&channel).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    echo(&channel).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(
        server.open(),
        1,
        "closed 1 s after the first call, not the last"
    );
    until("the idle session closed", || server.open() == 0).await;
    assert_eq!(server.connections(), 1);
}

/// The dial a call started outlives the call, and still counts: the session it opens is closed
/// once idle, and not left open with nothing to start the timer.
#[tokio::test]
async fn a_session_whose_caller_gave_up_on_its_dial_is_closed_once_idle() {
    let pki = Pki::new();
    let server =
        TlsServer::answering_after(pki.server(&["127.0.0.1"]), None, Duration::from_millis(500))
            .await;
    let mut transport =
        TransportConfig::new(Uri::try_from(server.endpoint.as_str()).expect("an endpoint"));
    transport.http2.idle_timeout = Some(Duration::from_millis(100));
    let mut tls = TlsConfig::default();
    tls.roots = vec![pki.root()];
    transport.tls = tls;
    let channel = channel_with(GrpcChannelConfig::new(transport)).expect("a channel");

    let mut options = CallStartOptions::new(ECHO);
    options.deadline = Some(Deadline::Timeout(Duration::from_millis(50)));
    let (_, _, status) = unary(&channel, options, Bytes::from_static(b"hello")).await;
    assert_eq!(status.code, GrpcStatusCode::DeadlineExceeded, "{status}");

    until("the dial landed", || server.open() == 1).await;
    until("the idle session closed", || server.open() == 0).await;
}
