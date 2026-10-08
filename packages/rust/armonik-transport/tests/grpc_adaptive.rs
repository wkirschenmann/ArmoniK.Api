//! What a channel does by the health of its server: retries stop while it fails more than it
//! accepts, first attempts wait at a cap while it is overloaded, and both lift when it accepts again.
//!
//! The windows are short, so that the tests take seconds: a window of `W` seconds counts a slot of a
//! twelfth of it.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use armonik_transport::grpc::{
    AdaptiveConfig, CallStartOptions, Cause, Deadline, GrpcChannel, GrpcChannelConfig,
    GrpcStatusCode, MetadataValue, RetryConfig,
};
use armonik_transport::hooks::AdaptiveState;
use armonik_transport::http2::TransportConfig;
use bytes::Bytes;
use common::echo::*;
use common::refuser::{Refusal, Refuser};
use http::Uri;

static KEYS: AtomicUsize = AtomicUsize::new(0);

/// A channel that retries as the default policy does, quickly, and judges its server as `adaptive`
/// says.
fn judging(endpoint: &str, adaptive: Option<AdaptiveConfig>) -> GrpcChannel {
    let mut config = GrpcChannelConfig::new(TransportConfig::new(
        Uri::try_from(endpoint).expect("an endpoint"),
    ));
    let mut retry = RetryConfig::default();
    retry.initial_backoff = Duration::from_millis(2);
    retry.max_backoff = Duration::from_millis(5);
    config.retry = Some(retry);
    config.adaptive = adaptive;
    channel_with(config).expect("a channel")
}

/// A channel with no retry policy, that judges its server as `adaptive` says.
fn judging_once(endpoint: &str, adaptive: Option<AdaptiveConfig>) -> GrpcChannel {
    let mut config = GrpcChannelConfig::new(TransportConfig::new(
        Uri::try_from(endpoint).expect("an endpoint"),
    ));
    config.retry = None;
    config.adaptive = adaptive;
    channel_with(config).expect("a channel")
}

/// A judgment with a slack of five, a window of ten seconds and a floor of two a second.
fn adaptive() -> AdaptiveConfig {
    let mut config = AdaptiveConfig::default();
    config.slack = 5;
    config.window = Duration::from_secs(10);
    config.floor_per_second = 2.0;
    config
}

/// A call to the flaky method that fails `times` times, with `code` and what `extra` adds, under a
/// key of its own, and its options.
fn flaky(code: &str, times: usize, extra: &[(&str, &str)]) -> (String, CallStartOptions) {
    let key = format!("adaptive-{}", KEYS.fetch_add(1, Ordering::SeqCst));
    let mut options = CallStartOptions::new(FLAKY);
    let times = times.to_string();
    for (name, value) in [
        ("x-flaky-key", key.as_str()),
        ("x-fail-times", times.as_str()),
        ("x-fail-code", code),
    ]
    .into_iter()
    .chain(extra.iter().copied())
    {
        options
            .metadata
            .append(name, MetadataValue::Ascii(value.to_owned()))
            .expect("a header");
    }
    (key, options)
}

/// A call to the flaky method that always fails: the key it is made under, and what it ended with.
async fn failing(
    channel: &GrpcChannel,
    code: &str,
    extra: &[(&str, &str)],
) -> (String, GrpcStatusCode) {
    let (key, options) = flaky(code, 1_000, extra);
    let (_, _, status) = unary(channel, options, Bytes::from_static(b"x")).await;
    (key, status.code)
}

async fn healthy(channel: &GrpcChannel) {
    let (_, _, status) = unary(
        channel,
        CallStartOptions::new(ECHO),
        Bytes::from_static(b"x"),
    )
    .await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
}

fn reading(channel: &GrpcChannel) -> AdaptiveState {
    channel.adaptive_state().expect("a channel that judges")
}

#[tokio::test]
async fn retries_stop_while_the_server_fails_more_than_it_accepts_and_open_again_when_it_accepts() {
    let server = TestServer::start().await;
    let channel = judging(&server.endpoint, Some(adaptive()));

    // The first call is retried to its maximum: five attempts, one over the slack of five only when
    // the last has ended.
    let (first, code) = failing(&channel, "14", &[]).await;
    assert_eq!(code, GrpcStatusCode::Unavailable);
    assert_eq!(flaky_seen(&first).len(), 5);
    assert!(
        reading(&channel).retries_open,
        "five attempts are the slack"
    );

    // The next fails once and is not retried, since the sixth attempt put the channel over.
    let (second, _) = failing(&channel, "14", &[]).await;
    assert_eq!(
        flaky_seen(&second).len(),
        1,
        "the retry reading is over the slack"
    );
    let closed = reading(&channel);
    assert!(!closed.retries_open);
    assert_eq!(
        closed.cap_per_second, None,
        "a transient failure never caps the rate"
    );

    // Healthy attempts reopen it: six failures and five accepts leave R - 2A at 1.
    for _ in 0..5 {
        healthy(&channel).await;
    }
    assert!(reading(&channel).retries_open, "{:?}", reading(&channel));
    let (third, _) = failing(&channel, "14", &[]).await;
    assert!(flaky_seen(&third).len() > 1, "retries are sent again");
}

#[tokio::test]
async fn a_channel_that_judges_nothing_retries_whatever_the_server_does() {
    let server = TestServer::start().await;
    let channel = judging(&server.endpoint, None);

    for _ in 0..4 {
        let (key, _) = failing(&channel, "14", &[]).await;
        assert_eq!(flaky_seen(&key).len(), 5);
    }
    assert!(channel.adaptive_state().is_none());
}

#[tokio::test]
async fn an_overloaded_server_caps_the_first_attempts_which_wait_in_order_and_probe_it() {
    let server = TestServer::start().await;
    let channel = judging(&server.endpoint, Some(adaptive()));

    // RESOURCE_EXHAUSTED is not retried, and it is overload: with nothing accepted the rate is
    // capped past the slack of five.
    for _ in 0..6 {
        let (_, code) = failing(&channel, "8", &[]).await;
        assert_eq!(code, GrpcStatusCode::ResourceExhausted);
    }
    let capped = reading(&channel);
    assert_eq!(capped.overloaded, 6);
    assert_eq!(
        capped.cap_per_second,
        Some(2.0),
        "the floor, with nothing accepted"
    );

    // Six more calls are paced at two a second, two at once: they take about two seconds, and the
    // calls end in the order they were made.
    let started = Instant::now();
    let order = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut calls = Vec::new();
    for index in 0..6 {
        let channel = channel.clone();
        let order = order.clone();
        let (key, options) = flaky("8", 1_000, &[]);
        calls.push(tokio::spawn(async move {
            let (_, _, status) = unary(&channel, options, Bytes::from_static(b"x")).await;
            order.lock().expect("the order").push((index, key));
            status.code
        }));
        // Calls reach the cap in the order they were made.
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    for call in calls {
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(30), call)
                .await
                .expect("the call ended")
                .expect("the task ran"),
            GrpcStatusCode::ResourceExhausted
        );
    }
    let took = started.elapsed();
    assert!(took >= Duration::from_millis(1400), "{took:?}");
    assert!(took < Duration::from_secs(10), "{took:?}");
    let order = order.lock().expect("the order");
    let indexes: Vec<_> = order.iter().map(|(index, _)| *index).collect();
    assert_eq!(
        indexes,
        vec![0, 1, 2, 3, 4, 5],
        "served in the order they arrived"
    );
}

#[tokio::test]
async fn a_call_waiting_at_the_cap_ends_at_its_deadline_having_sent_nothing() {
    let server = TestServer::start().await;
    let channel = judging(&server.endpoint, Some(adaptive()));
    for _ in 0..6 {
        failing(&channel, "8", &[]).await;
    }
    // The burst of the cap, two turns, goes first.
    for _ in 0..2 {
        failing(&channel, "8", &[]).await;
    }

    let (key, mut options) = flaky("8", 1_000, &[]);
    options.deadline = Some(Deadline::Timeout(Duration::from_millis(100)));
    let (_, _, status) = unary(&channel, options, Bytes::from_static(b"x")).await;

    assert_eq!(status.code, GrpcStatusCode::DeadlineExceeded, "{status}");
    assert_eq!(flaky_seen(&key).len(), 0, "nothing was sent");
}

/// Accepted attempts age the overload out, and the cap lifts within a window of the end.
#[tokio::test]
async fn the_cap_lifts_once_the_server_accepts_again() {
    let server = TestServer::start().await;
    let mut config = adaptive();
    config.window = Duration::from_millis(1200);
    let channel = judging(&server.endpoint, Some(config));

    for _ in 0..6 {
        failing(&channel, "8", &[]).await;
    }
    assert!(reading(&channel).cap_per_second.is_some());

    // Probes at the cap go through, and the overloaded attempts leave the window.
    let started = Instant::now();
    while reading(&channel).cap_per_second.is_some() {
        healthy(&channel).await;
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "{:?}",
            reading(&channel)
        );
    }
    // And first attempts start at once again.
    let before = Instant::now();
    for _ in 0..5 {
        healthy(&channel).await;
    }
    assert!(
        before.elapsed() < Duration::from_secs(2),
        "{:?}",
        before.elapsed()
    );
}

/// The lists are the caller's: a server whose outage is its way to say it is full is judged so by
/// naming UNAVAILABLE as overload, and one whose failures are named nowhere is accepted.
#[tokio::test]
async fn the_lists_decide_what_each_failure_counts_as() {
    let server = TestServer::start().await;

    let mut config = adaptive();
    config
        .transient
        .retain(|cause| *cause != Cause::Status(GrpcStatusCode::Unavailable));
    config
        .overload
        .push(Cause::Status(GrpcStatusCode::Unavailable));
    let channel = judging_once(&server.endpoint, Some(config));
    for _ in 0..6 {
        failing(&channel, "14", &[]).await;
    }
    let state = reading(&channel);
    assert_eq!((state.overloaded, state.transient), (6, 0), "{state:?}");
    assert!(state.cap_per_second.is_some(), "{state:?}");
    assert!(!state.retries_open);

    let mut config = adaptive();
    config.transient.clear();
    config.overload.clear();
    let channel = judging_once(&server.endpoint, Some(config));
    for _ in 0..12 {
        failing(&channel, "14", &[]).await;
    }
    let state = reading(&channel);
    assert_eq!(
        (state.accepted, state.transient, state.overloaded),
        (12, 0, 0),
        "a server failure that no list names is an acceptance: {state:?}"
    );
    assert!(state.retries_open && state.cap_per_second.is_none());
}

#[tokio::test]
async fn a_pushback_is_overload_whatever_the_code() {
    let server = TestServer::start().await;
    let channel = judging(&server.endpoint, Some(adaptive()));

    // An ABORTED with a pushback of a few milliseconds: not a retried code, and overload.
    for _ in 0..6 {
        failing(&channel, "10", &[("x-pushback", "3")]).await;
    }
    let state = reading(&channel);
    assert_eq!(state.overloaded, 6, "{state:?}");
    assert!(state.cap_per_second.is_some());
}

/// A proxy's 429 is overload, its 503 an outage, and its 404 an answer.
#[tokio::test]
async fn a_proxys_answer_is_counted_by_its_http_status() {
    let server = TestServer::start().await;
    let channel = judging_once(&server.endpoint, Some(adaptive()));
    let call = |status: &'static str| {
        let channel = channel.clone();
        async move {
            let mut options = CallStartOptions::new("/raw/HttpError");
            options
                .metadata
                .append("x-http-status", MetadataValue::Ascii(status.to_owned()))
                .expect("a header");
            unary(&channel, options, Bytes::from_static(b"x")).await;
        }
    };

    call("429").await;
    call("503").await;
    call("404").await;
    call("200").await;
    let state = reading(&channel);
    assert_eq!(
        (state.overloaded, state.transient, state.accepted),
        (1, 1, 1),
        "the 200 counts nothing: {state:?}"
    );
}

/// A refused stream is overload, and a stream a GOAWAY left unsent counts nothing.
#[tokio::test]
async fn a_refused_stream_is_overload_and_a_goaway_counts_nothing() {
    let refused = Refuser::start(Refusal::RefusedStream, 1).await;
    let channel = judging(&refused.endpoint, Some(adaptive()));
    healthy(&channel).await;
    let state = reading(&channel);
    assert_eq!((state.overloaded, state.accepted), (1, 1), "{state:?}");

    let goaway = Refuser::start(Refusal::GoAway, 1).await;
    let channel = judging(&goaway.endpoint, Some(adaptive()));
    healthy(&channel).await;
    let state = reading(&channel);
    assert_eq!(
        (state.overloaded, state.transient, state.accepted),
        (0, 0, 1),
        "the GOAWAY and the resend count nothing: {state:?}"
    );
}

/// What the estimate holds is what was recorded, whatever the threads that recorded it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn many_threads_count_every_attempt_once() {
    let server = TestServer::start().await;
    let mut config = adaptive();
    config.window = Duration::from_secs(600);
    let channel = judging(&server.endpoint, Some(config));

    let mut tasks = Vec::new();
    for _ in 0..8 {
        let channel = channel.clone();
        tasks.push(tokio::spawn(async move {
            for _ in 0..25 {
                healthy(&channel).await;
            }
        }));
    }
    for task in tasks {
        task.await.expect("the task ran");
    }
    let state = reading(&channel);
    assert_eq!(state.accepted, 200, "{state:?}");
    assert!(state.retries_open && state.cap_per_second.is_none());
}
