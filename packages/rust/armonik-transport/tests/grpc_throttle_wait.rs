//! What a call does while it waits for its turn at the cap of the throttle: it ends when the
//! channel closes or the call is cancelled, the deadline the server is told is what is left after
//! the wait, and a request its peer never processed, sent again, takes a turn of its own.
//!
//! Each channel allows no failure (a slack of 0) and a throttle multiplier of 1, so that one
//! overload, a RESOURCE_EXHAUSTED answer or a refused stream, caps it, and what the server accepts
//! does not lift the cap. Until something is accepted the cap is the floor, a call in each
//! `spacing`: the first call that takes its turn leaves a debt of `spacing` for the next, and the
//! debt stays when the cap rises after an accept.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use armonik_transport::grpc::{
    CallStartOptions, Deadline, GrpcChannel, GrpcChannelConfig, GrpcStatus, GrpcStatusCode,
    MetadataValue,
};
use armonik_transport::http2::TransportConfig;
use bytes::Bytes;
use common::echo::*;
use common::refuser::{Refusal, Refuser};
use http::Uri;

static KEYS: AtomicUsize = AtomicUsize::new(0);

/// A channel to `endpoint` that an overload caps, to a first attempt in each `spacing` at first.
/// `spacing` is at least a second, so that the cap allows no burst.
fn allowing_no_failure(endpoint: &str, spacing: Duration) -> GrpcChannel {
    let mut config = GrpcChannelConfig::new(TransportConfig::new(
        Uri::try_from(endpoint).expect("an endpoint"),
    ));
    let mut adaptive = config.adaptive.take().expect("a judgment by default");
    adaptive.slack = 0;
    adaptive.throttle_multiplier = 1.0;
    adaptive.floor_per_second = 1.0 / spacing.as_secs_f64();
    config.adaptive = Some(adaptive);
    channel_with(config).expect("a channel")
}

/// Caps `channel` with one call the server answers RESOURCE_EXHAUSTED, which is overload.
async fn cap(channel: &GrpcChannel) {
    let mut options = CallStartOptions::new(FLAKY);
    let key = format!("throttle-wait-{}", KEYS.fetch_add(1, Ordering::SeqCst));
    for (name, value) in [
        ("x-flaky-key", key.as_str()),
        ("x-fail-times", "1000"),
        ("x-fail-code", "8"),
    ] {
        options
            .metadata
            .append(name, MetadataValue::Ascii(value.to_owned()))
            .expect("a header");
    }
    let (_, _, status) = unary(channel, options, Bytes::from_static(b"x")).await;
    assert_eq!(status.code, GrpcStatusCode::ResourceExhausted, "{status}");
    let state = channel.adaptive_state().expect("a channel that judges");
    assert!(state.cap_per_second.is_some(), "{state:?}");
}

/// One echo call, and how long it took from `since` to end.
async fn echo_call(
    channel: &GrpcChannel,
    options: CallStartOptions,
    since: Instant,
) -> (Duration, GrpcStatus) {
    let (mut send, mut recv, _control) = channel
        .start_call(options)
        .expect("the call starts")
        .split();
    let _ = send.send_message(Bytes::from_static(b"x")).await;
    let _ = send.end_send().await;
    let (_, _, status) = read_to_terminal(&mut recv).await;
    (since.elapsed(), status)
}

/// Fails the test if the call does not end within `limit`.
async fn bounded<T>(limit: Duration, call: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(limit, call)
        .await
        .expect("the call ended")
}

/// The `grpc-timeout` the server was sent, in seconds, from what `/raw/EchoHeaders` echoes.
fn sent_timeout_seconds(messages: &[Bytes]) -> f64 {
    let seen = String::from_utf8(messages.concat().to_vec()).expect("the headers as text");
    let value = seen
        .split_whitespace()
        .find_map(|pair| pair.strip_prefix("grpc-timeout="))
        .expect("a grpc-timeout");
    let (digits, unit) = value.split_at(value.len() - 1);
    let amount: f64 = digits.parse().expect("a number");
    match unit {
        "S" => amount,
        "m" => amount / 1e3,
        "u" => amount / 1e6,
        "n" => amount / 1e9,
        other => panic!("not a grpc-timeout unit: {other}"),
    }
}

#[tokio::test]
async fn a_call_waiting_at_the_cap_ends_cancelled_when_it_is_cancelled() {
    let server = TestServer::start().await;
    let channel = allowing_no_failure(&server.endpoint, Duration::from_secs(60));
    cap(&channel).await;

    // The first turn of the cap.
    let (_, status) = echo_call(&channel, CallStartOptions::new(ECHO), Instant::now()).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");

    let (mut send, mut recv, control) = channel
        .start_call(CallStartOptions::new(ECHO))
        .expect("the call starts")
        .split();
    let _ = send.send_message(Bytes::from_static(b"x")).await;
    let _ = send.end_send().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    control.cancel();
    ends_cancelled(&mut recv, "the cancel did not end the waiting call").await;
}

#[tokio::test]
async fn closing_the_channel_ends_the_calls_waiting_at_the_cap_cancelled() {
    let server = TestServer::start().await;
    let channel = allowing_no_failure(&server.endpoint, Duration::from_secs(60));
    cap(&channel).await;

    let (_, status) = echo_call(&channel, CallStartOptions::new(ECHO), Instant::now()).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");

    let (mut send, mut recv, _control) = channel
        .start_call(CallStartOptions::new(ECHO))
        .expect("the call starts")
        .split();
    let _ = send.send_message(Bytes::from_static(b"x")).await;
    let _ = send.end_send().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    channel.close();
    ends_cancelled(
        &mut recv,
        "closing the channel did not end the waiting call",
    )
    .await;
}

/// What the server is told is what is left of the deadline once the wait is over.
#[tokio::test]
async fn the_deadline_the_server_is_told_is_what_is_left_after_the_wait() {
    let server = TestServer::start().await;
    let spacing = Duration::from_secs(4);
    let channel = allowing_no_failure(&server.endpoint, spacing);
    cap(&channel).await;

    let (_, status) = echo_call(&channel, CallStartOptions::new(ECHO), Instant::now()).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");

    let waited = Instant::now();
    let mut options = CallStartOptions::new("/raw/EchoHeaders");
    options.deadline = Some(Deadline::Timeout(Duration::from_secs(30)));
    let (mut send, mut recv, _control) = channel
        .start_call(options)
        .expect("the call starts")
        .split();
    let _ = send.send_message(Bytes::from_static(b"x")).await;
    let _ = send.end_send().await;
    let (_, messages, status) = bounded(Duration::from_secs(30), read_to_terminal(&mut recv)).await;

    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    // The second call waited for most of a spacing, which the 30 s did not count.
    assert!(waited.elapsed() > spacing / 2, "it did not wait");
    let sent = sent_timeout_seconds(&messages);
    assert!(
        sent < 30.0 - waited.elapsed().as_secs_f64() / 2.0,
        "{sent} s were stated after a wait of {:?}",
        waited.elapsed()
    );
}

/// A request its peer never processed is sent again whatever the policy, and that is a first
/// attempt of its own: it waits for a turn of the cap as one does.
///
/// The refuser turns away the first three streams. A refusal is overload, so the first call caps
/// the channel, with no call to `cap`; its resend takes the first turn of the cap, and its second
/// refusal ends it. The second call takes the next turn, is refused, and its resend owes a spacing
/// again.
#[tokio::test]
async fn a_transparent_resend_waits_for_a_turn_of_its_own() {
    let refuser = Refuser::start(Refusal::RefusedStream, 3).await;
    let spacing = Duration::from_secs(1);
    let channel = allowing_no_failure(&refuser.endpoint, spacing);

    let (_, status) = bounded(
        Duration::from_secs(30),
        echo_call(&channel, CallStartOptions::new(ECHO), Instant::now()),
    )
    .await;
    assert_eq!(
        status.code,
        GrpcStatusCode::Unavailable,
        "the second refusal is not the resend's to retry: {status}"
    );
    assert_eq!(refuser.seen().len(), 2);

    let (took, status) = bounded(
        Duration::from_secs(30),
        echo_call(&channel, CallStartOptions::new(ECHO), Instant::now()),
    )
    .await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(refuser.seen().len(), 4);
    // The second call's first attempt waited for the turn after the first call's resend, and its
    // resend for the turn after that.
    assert!(took >= spacing * 3 / 2, "{took:?}");
}
