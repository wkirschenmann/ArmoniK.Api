//! The channel's rate limit: how many requests start in a window, and what a request over it does
//! while it waits for the next.

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use armonik_transport::grpc::{
    CallStartOptions, Deadline, GrpcChannel, GrpcChannelConfig, GrpcChannelConfigError, GrpcStatus,
    GrpcStatusCode, HeadOrigin, MetadataValue, RateLimitConfig, RetryConfig,
};
use armonik_transport::http2::TransportConfig;
use bytes::Bytes;
use common::echo::*;
use common::refuser::{Refusal, Refuser};
use http::Uri;

fn limited(endpoint: &str, calls: usize, per: Duration) -> GrpcChannel {
    let mut config = GrpcChannelConfig::new(TransportConfig::new(
        Uri::try_from(endpoint).expect("an endpoint"),
    ));
    config.rate_limit = Some(RateLimitConfig::new(calls, per));
    channel_with(config).expect("a channel")
}

/// One echo call, and when it ended, counted from `since`.
async fn echo_call(
    channel: &GrpcChannel,
    options: CallStartOptions,
    since: Instant,
) -> (Duration, GrpcStatus, HeadOrigin) {
    let (mut send, mut recv, _control) = channel
        .start_call(options)
        .expect("the call starts")
        .split();
    let _ = send.send_message(Bytes::from_static(b"x")).await;
    let _ = send.end_send().await;
    let origin = recv.recv_head().await.expect("a head").origin;
    let (_, _, status) = read_to_terminal(&mut recv).await;
    (since.elapsed(), status, origin)
}

fn with_deadline(after: Duration) -> CallStartOptions {
    let mut options = CallStartOptions::new(ECHO);
    options.deadline = Some(Deadline::Timeout(after));
    options
}

/// A call that ends within `limit`, which a waiting call that nothing ends would not.
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
async fn requests_over_the_limit_start_in_the_next_window() {
    let server = TestServer::start().await;
    let window = Duration::from_secs(1);
    let channel = limited(&server.endpoint, 2, window);

    let since = Instant::now();
    let mut calls = Vec::new();
    for _ in 0..5 {
        let channel = channel.clone();
        calls.push(tokio::spawn(async move {
            echo_call(&channel, CallStartOptions::new(ECHO), since).await
        }));
    }
    let mut ended = Vec::new();
    for call in calls {
        let (at, status, _) = bounded(Duration::from_secs(30), call)
            .await
            .expect("the task ran");
        assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
        ended.push(at);
    }
    ended.sort();

    // Two in the first window, two in the second, which the third request opens when the first
    // ends, and the last in the third.
    assert!(ended[1] < window, "{ended:?}");
    assert!(ended[2] >= window && ended[3] >= window, "{ended:?}");
    assert!(ended[3] < 2 * window, "{ended:?}");
    assert!(ended[4] >= 2 * window, "{ended:?}");
}

#[tokio::test]
async fn requests_start_in_the_order_they_were_made() {
    let server = TestServer::start().await;
    let channel = limited(&server.endpoint, 1, Duration::from_millis(300));

    let order = Arc::new(Mutex::new(Vec::new()));
    let since = Instant::now();
    let mut calls = Vec::new();
    for index in 0..5 {
        let (mut send, mut recv, _control) = channel
            .start_call(CallStartOptions::new(ECHO))
            .expect("the call starts")
            .split();
        let order = order.clone();
        calls.push(tokio::spawn(async move {
            let _ = send.send_message(Bytes::from_static(b"x")).await;
            let _ = send.end_send().await;
            let (_, _, status) = read_to_terminal(&mut recv).await;
            assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
            order.lock().expect("the order").push(index);
        }));
    }
    for call in calls {
        bounded(Duration::from_secs(30), call)
            .await
            .expect("the task ran");
    }

    assert_eq!(*order.lock().expect("the order"), vec![0, 1, 2, 3, 4]);
    assert!(since.elapsed() >= Duration::from_millis(1200));
}

#[tokio::test]
async fn a_waiting_call_ends_deadline_exceeded_at_its_deadline_having_sent_nothing() {
    let server = TestServer::start().await;
    let channel = limited(&server.endpoint, 1, Duration::from_secs(60));

    let since = Instant::now();
    let (_, status, _) = echo_call(&channel, CallStartOptions::new(ECHO), since).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");

    let waiting = Instant::now();
    let (at, status, origin) = bounded(
        Duration::from_secs(30),
        echo_call(&channel, with_deadline(Duration::from_millis(300)), waiting),
    )
    .await;
    assert_eq!(status.code, GrpcStatusCode::DeadlineExceeded, "{status}");
    assert_eq!(origin, HeadOrigin::NoResponse);
    assert!(at >= Duration::from_millis(300), "{at:?}");
    assert!(at < Duration::from_secs(30), "{at:?}");
}

#[tokio::test]
async fn a_waiting_call_ends_cancelled_when_it_is_cancelled() {
    let server = TestServer::start().await;
    let channel = limited(&server.endpoint, 1, Duration::from_secs(60));

    let since = Instant::now();
    let (_, status, _) = echo_call(&channel, CallStartOptions::new(ECHO), since).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");

    let (_send, mut recv, control) = channel
        .start_call(CallStartOptions::new(ECHO))
        .expect("the call starts")
        .split();
    tokio::time::sleep(Duration::from_millis(200)).await;
    control.cancel();
    ends_cancelled(&mut recv, "the cancel did not end the waiting call").await;
}

#[tokio::test]
async fn closing_the_channel_ends_the_waiting_calls_cancelled() {
    let server = TestServer::start().await;
    let channel = limited(&server.endpoint, 1, Duration::from_secs(60));

    let since = Instant::now();
    let (_, status, _) = echo_call(&channel, CallStartOptions::new(ECHO), since).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");

    let (_send, mut recv, _control) = channel
        .start_call(CallStartOptions::new(ECHO))
        .expect("the call starts")
        .split();
    tokio::time::sleep(Duration::from_millis(200)).await;
    channel.close();
    ends_cancelled(
        &mut recv,
        "closing the channel did not end the waiting call",
    )
    .await;
}

/// A call that stopped waiting leaves the next one waiting no longer than the window, rather than
/// holding the turn it never took.
#[tokio::test]
async fn a_call_that_stopped_waiting_takes_no_turn_and_holds_none_back() {
    let server = TestServer::start().await;
    let window = Duration::from_millis(1500);
    let channel = limited(&server.endpoint, 1, window);

    let since = Instant::now();
    let (_, status, _) = echo_call(&channel, CallStartOptions::new(ECHO), since).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");

    // Second in line, gone before its turn.
    let (_, status, _) = echo_call(
        &channel,
        with_deadline(Duration::from_millis(200)),
        Instant::now(),
    )
    .await;
    assert_eq!(status.code, GrpcStatusCode::DeadlineExceeded, "{status}");

    let (at, status, _) = bounded(
        Duration::from_secs(30),
        echo_call(&channel, CallStartOptions::new(ECHO), since),
    )
    .await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert!(at >= window, "{at:?}");
    assert!(at < 2 * window, "{at:?}");
}

/// What the server is told is what is left of the deadline once the wait is over.
#[tokio::test]
async fn the_deadline_the_server_is_told_is_what_is_left_after_the_wait() {
    let server = TestServer::start().await;
    let channel = limited(&server.endpoint, 1, Duration::from_secs(2));

    let (_, status, _) = echo_call(&channel, CallStartOptions::new(ECHO), Instant::now()).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");

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
    // The second request waited for most of a 2 s window, which the 30 s did not count.
    assert!(sent_timeout_seconds(&messages) < 29.5);
}

/// A request its peer never processed is sent again whatever the policy, and that is a request.
#[tokio::test]
async fn a_transparent_resend_is_a_request_of_its_own() {
    let refuser = Refuser::start(Refusal::RefusedStream, 1).await;
    let window = Duration::from_millis(800);
    let mut config = GrpcChannelConfig::new(TransportConfig::new(
        Uri::try_from(refuser.endpoint.as_str()).expect("an endpoint"),
    ));
    config.rate_limit = Some(RateLimitConfig::new(1, window));
    // One attempt retries nothing the policy counts; the resend is not one.
    let mut retry = RetryConfig::default();
    retry.max_attempts = 1;
    config.retry = Some(retry);
    let channel = channel_with(config).expect("a channel");

    let (at, status, _) = bounded(
        Duration::from_secs(30),
        echo_call(&channel, CallStartOptions::new(ECHO), Instant::now()),
    )
    .await;

    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(refuser.seen().len(), 2);
    assert!(at >= window, "{at:?}");
}

#[tokio::test]
async fn a_retry_is_a_request_of_its_own() {
    let server = TestServer::start().await;
    let window = Duration::from_millis(500);
    let mut config = GrpcChannelConfig::new(TransportConfig::new(
        Uri::try_from(server.endpoint.as_str()).expect("an endpoint"),
    ));
    config.rate_limit = Some(RateLimitConfig::new(1, window));
    let mut retry = RetryConfig::default();
    retry.initial_backoff = Duration::from_millis(10);
    retry.max_backoff = Duration::from_millis(50);
    config.retry = Some(retry);
    let channel = channel_with(config).expect("a channel");

    let mut options = CallStartOptions::new(FLAKY);
    for (name, value) in [("x-flaky-key", "rate-limited"), ("x-fail-times", "2")] {
        options
            .metadata
            .append(name, MetadataValue::Ascii(value.to_owned()))
            .expect("a header");
    }
    let (at, status, _) = bounded(
        Duration::from_secs(30),
        echo_call(&channel, options, Instant::now()),
    )
    .await;

    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(flaky_seen("rate-limited").len(), 3);
    // One request per window: the second attempt opens the second, the third the third.
    assert!(at >= 2 * window, "{at:?}");
}

/// A streaming call is one request, however many messages it exchanges.
#[tokio::test]
async fn a_stream_is_one_request_however_many_messages_it_carries() {
    let server = TestServer::start().await;
    let channel = limited(&server.endpoint, 1, Duration::from_secs(3));

    let started = Instant::now();
    let (mut send, mut recv, _control) = channel
        .start_call(CallStartOptions::new(COLLECT))
        .expect("the call starts")
        .split();
    for message in ["a", "b", "c", "d", "e"] {
        send.send_message(Bytes::from(message))
            .await
            .expect("a message goes");
    }
    send.end_send().await.expect("the end goes");
    let (_, messages, status) = bounded(Duration::from_secs(30), read_to_terminal(&mut recv)).await;

    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(messages, vec![Bytes::from_static(b"5:a,b,c,d,e")]);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn the_limit_is_each_channels_own() {
    let server = TestServer::start().await;
    let first = limited(&server.endpoint, 1, Duration::from_secs(60));
    let second = limited(&server.endpoint, 1, Duration::from_secs(60));

    for channel in [&first, &second] {
        let (_, status, _) = bounded(
            Duration::from_secs(30),
            echo_call(channel, CallStartOptions::new(ECHO), Instant::now()),
        )
        .await;
        assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    }
}

#[tokio::test]
async fn a_limit_that_starts_nothing_is_refused() {
    let server = TestServer::start().await;
    for (calls, per) in [(0, Duration::from_secs(1)), (1, Duration::ZERO)] {
        let mut config = GrpcChannelConfig::new(TransportConfig::new(
            Uri::try_from(server.endpoint.as_str()).expect("an endpoint"),
        ));
        config.rate_limit = Some(RateLimitConfig::new(calls, per));
        assert!(
            matches!(
                channel_with(config),
                Err(GrpcChannelConfigError::RateLimit { .. })
            ),
            "{calls} per {per:?}"
        );
    }
}
