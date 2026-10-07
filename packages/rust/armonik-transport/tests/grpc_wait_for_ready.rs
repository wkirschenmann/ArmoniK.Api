//! A call that waits for the channel to be ready: where it waits, what it waits for, and how the
//! wait ends.

mod common;

use std::time::{Duration, Instant};

use armonik_transport::grpc::{
    CallStartOptions, Deadline, GrpcChannel, GrpcStatus, GrpcStatusCode, MetadataValue,
};
use bytes::Bytes;
use common::echo::*;

fn waiting(method: &str) -> CallStartOptions {
    let mut options = CallStartOptions::new(method);
    options.wait_for_ready = true;
    options
}

/// The echo of one message, as the call ends: what it says of the server it reached.
async fn echoed(channel: &GrpcChannel, options: CallStartOptions) -> (Vec<Bytes>, GrpcStatus) {
    let (_, messages, status) = unary(channel, options, Bytes::from_static(b"x")).await;
    (messages, status)
}

/// A call the test's own timer bounds, so that a wait that never ends fails the test rather than
/// hanging it.
async fn within<T>(call: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(30), call)
        .await
        .expect("the call ended")
}

#[tokio::test]
async fn a_call_that_does_not_wait_ends_unavailable_where_the_server_is_down() {
    let channel = channel(&closed_port().await);

    let (_, status) = within(echoed(&channel, CallStartOptions::new(ECHO))).await;
    assert_eq!(status.code, GrpcStatusCode::Unavailable, "{status}");
}

#[tokio::test]
async fn a_call_that_waits_goes_out_once_the_server_is_up() {
    let endpoint = closed_port().await;
    let channel = channel(&endpoint);
    let call = tokio::spawn({
        let channel = channel.clone();
        async move { echoed(&channel, waiting(ECHO)).await }
    });

    // Long enough for a refused dial to be told, wherever the platform takes its time over one,
    // so that the call is usually in its backoff when the server comes up.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert!(!call.is_finished(), "the call waits where it would fail");
    let server = TestServer::start_at(&endpoint).await;

    let (messages, status) = within(call).await.expect("the call task");
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(messages, vec![Bytes::from_static(b"x")]);
    assert_eq!(server.connections(), 1);
}

/// The calls that wait share the channel's dial and its backoff: one connection serves them all.
#[tokio::test]
async fn calls_that_wait_share_one_dial() {
    let endpoint = closed_port().await;
    let channel = channel(&endpoint);
    let calls: Vec<_> = (0..3)
        .map(|_| {
            let channel = channel.clone();
            tokio::spawn(async move { echoed(&channel, waiting(ECHO)).await })
        })
        .collect();

    tokio::time::sleep(Duration::from_millis(300)).await;
    let server = TestServer::start_at(&endpoint).await;

    for call in calls {
        let (_, status) = within(call).await.expect("the call task");
        assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    }
    assert_eq!(server.connections(), 1);
}

/// A call that does not wait still fails at once, on the channel a call that waits is waiting on.
#[tokio::test]
async fn a_call_that_does_not_wait_fails_beside_one_that_does() {
    let endpoint = closed_port().await;
    let channel = channel(&endpoint);
    let call = tokio::spawn({
        let channel = channel.clone();
        async move { echoed(&channel, waiting(ECHO)).await }
    });
    tokio::time::sleep(Duration::from_millis(200)).await;

    let (_, status) = within(echoed(&channel, CallStartOptions::new(ECHO))).await;
    assert_eq!(status.code, GrpcStatusCode::Unavailable, "{status}");
    assert!(!call.is_finished());

    let _server = TestServer::start_at(&endpoint).await;
    let (_, status) = within(call).await.expect("the call task");
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
}

/// A session another call opens ends the wait before the backoff does.
#[tokio::test]
async fn a_session_another_call_opens_ends_the_wait() {
    let endpoint = closed_port().await;
    let channel = channel(&endpoint);

    // A dial that failed, so that the channel is in its backoff, at least 800 ms.
    let (_, status) = within(echoed(&channel, CallStartOptions::new(ECHO))).await;
    assert_eq!(status.code, GrpcStatusCode::Unavailable, "{status}");

    let started = Instant::now();
    let call = tokio::spawn({
        let channel = channel.clone();
        async move { echoed(&channel, waiting(ECHO)).await }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!call.is_finished(), "the call waits out the backoff");

    // A call that does not wait dials at once, whatever the backoff.
    let server = TestServer::start_at(&endpoint).await;
    let (_, status) = within(echoed(&channel, CallStartOptions::new(ECHO))).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");

    let (_, status) = within(call).await.expect("the call task");
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert!(
        started.elapsed() < Duration::from_millis(750),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(server.connections(), 1);
}

#[tokio::test]
async fn a_call_that_waits_ends_deadline_exceeded_at_its_deadline() {
    let channel = channel(&closed_port().await);
    let mut options = waiting(ECHO);
    options.deadline = Some(Deadline::Timeout(Duration::from_millis(500)));

    let started = Instant::now();
    let (_, status) = within(echoed(&channel, options)).await;
    assert_eq!(status.code, GrpcStatusCode::DeadlineExceeded, "{status}");
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(450) && waited < Duration::from_secs(5),
        "{waited:?}"
    );
}

#[tokio::test]
async fn a_call_that_waits_ends_cancelled_when_it_is_cancelled() {
    let channel = channel(&closed_port().await);
    let (mut send, mut recv, control) = channel
        .start_call(waiting(ECHO))
        .expect("the call starts")
        .split();
    let _ = send.send_message(Bytes::from_static(b"x")).await;
    let _ = send.end_send().await;

    tokio::time::sleep(Duration::from_millis(300)).await;
    control.cancel();

    ends_cancelled(&mut recv, "the cancel ended the wait").await;
}

#[tokio::test]
async fn a_call_that_waits_ends_cancelled_when_its_channel_closes() {
    let channel = channel(&closed_port().await);
    let (mut send, mut recv, _control) = channel
        .start_call(waiting(ECHO))
        .expect("the call starts")
        .split();
    let _ = send.send_message(Bytes::from_static(b"x")).await;
    let _ = send.end_send().await;

    tokio::time::sleep(Duration::from_millis(300)).await;
    channel.close();

    ends_cancelled(&mut recv, "closing the channel ended the wait").await;
}

/// The wait is for a connection: a server that answers UNAVAILABLE has been reached, and its
/// answer is the call's.
#[tokio::test]
async fn a_call_that_reached_its_server_ends_with_what_the_server_says() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    let mut options = waiting(FLAKY);
    for (name, value) in [("x-flaky-key", "answered"), ("x-fail-times", "10")] {
        options
            .metadata
            .append(name, MetadataValue::Ascii(value.to_owned()))
            .expect("a header");
    }
    let (_, status) = within(echoed(&channel, options)).await;
    assert_eq!(status.code, GrpcStatusCode::Unavailable, "{status}");
    assert_eq!(flaky_seen("answered").len(), 1, "and was not sent again");
}

/// A server that is up is not waited for.
#[tokio::test]
async fn a_call_that_waits_goes_out_at_once_where_the_server_is_up() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    let started = Instant::now();
    let (messages, status) = within(echoed(&channel, waiting(ECHO))).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(messages, vec![Bytes::from_static(b"x")]);
    assert!(started.elapsed() < Duration::from_millis(700));
}
