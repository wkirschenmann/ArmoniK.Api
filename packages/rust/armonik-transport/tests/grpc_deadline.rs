//! A call's deadline: kept here, told to the server as `grpc-timeout`, and the channel's default
//! where the call names none.

mod common;

use std::time::{Duration, Instant};

use armonik_transport::grpc::{
    CallStartOptions, Deadline, GrpcChannel, GrpcChannelConfig, GrpcStatus, GrpcStatusCode,
    HeadOrigin,
};
use armonik_transport::http2::TransportConfig;
use bytes::Bytes;
use common::echo::*;
use http::Uri;

async fn with(
    channel: &GrpcChannel,
    method: &str,
    deadline: Option<Deadline>,
) -> (Vec<Bytes>, GrpcStatus, HeadOrigin) {
    let mut options = CallStartOptions::new(method);
    options.deadline = deadline;
    let (mut send, mut recv, _control) = channel
        .start_call(options)
        .expect("the call starts")
        .split();
    // A call over before its first send refuses it, which a past deadline makes certain.
    let _ = send.send_message(Bytes::from_static(b"x")).await;
    let _ = send.end_send().await;
    let origin = recv.recv_head().await.expect("a head").origin;
    let (_, messages, status) = read_to_terminal(&mut recv).await;
    (messages, status, origin)
}

/// The `grpc-timeout` the server was sent, as a duration; none when it was sent none.
fn sent_timeout(messages: &[Bytes]) -> Option<Duration> {
    let seen = String::from_utf8(messages.concat().to_vec()).expect("the headers as text");
    let value = seen
        .split_whitespace()
        .find_map(|pair| pair.strip_prefix("grpc-timeout="))?;
    let (digits, unit) = value.split_at(value.len() - 1);
    assert!(digits.len() <= 8, "more than eight digits: {value}");
    let amount: u64 = digits.parse().expect("a number");
    Some(match unit {
        "H" => Duration::from_secs(amount * 3600),
        "M" => Duration::from_secs(amount * 60),
        "S" => Duration::from_secs(amount),
        "m" => Duration::from_millis(amount),
        "u" => Duration::from_micros(amount),
        "n" => Duration::from_nanos(amount),
        other => panic!("not a grpc-timeout unit: {other}"),
    })
}

/// A call the deadline has to end, given far longer than its deadline and far shorter than the
/// hour `SLOW` takes to answer.
async fn bounded<T>(call: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(30), call)
        .await
        .expect("the deadline ended the call")
}

fn defaulting_to(endpoint: &str, default: Duration) -> GrpcChannel {
    let mut config = GrpcChannelConfig::new(TransportConfig::new(
        Uri::try_from(endpoint).expect("an endpoint"),
    ));
    config.default_deadline = Some(default);
    channel_with(config).expect("a channel")
}

#[tokio::test]
async fn a_call_past_its_deadline_ends_deadline_exceeded_without_waiting_for_the_server() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    let (_, status, _) = bounded(with(
        &channel,
        SLOW,
        Some(Deadline::Timeout(Duration::from_millis(200))),
    ))
    .await;
    assert_eq!(status.code, GrpcStatusCode::DeadlineExceeded, "{status}");

    // The stream is reset, not left open: the session takes the next call.
    let (messages, status, _) = with(&channel, ECHO, None).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(messages, vec![Bytes::from_static(b"x")]);
    assert_eq!(server.connections(), 1);
}

#[tokio::test]
async fn the_deadline_is_sent_as_grpc_timeout_and_no_deadline_sends_none() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    let (messages, status, _) = with(
        &channel,
        "/raw/EchoHeaders",
        Some(Deadline::Timeout(Duration::from_secs(30))),
    )
    .await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    let sent = sent_timeout(&messages).expect("a grpc-timeout");
    assert!(
        sent <= Duration::from_secs(30) && sent > Duration::from_secs(25),
        "{sent:?}"
    );

    let (messages, _, _) = with(&channel, "/raw/EchoHeaders", None).await;
    assert_eq!(sent_timeout(&messages), None);
}

#[tokio::test]
async fn the_channels_default_applies_unless_the_call_names_its_own() {
    let server = TestServer::start().await;
    let channel = defaulting_to(&server.endpoint, Duration::from_millis(200));

    let (_, status, _) = bounded(with(&channel, SLOW, None)).await;
    assert_eq!(status.code, GrpcStatusCode::DeadlineExceeded, "{status}");

    let (messages, _, _) = with(&channel, "/raw/EchoHeaders", None).await;
    let sent = sent_timeout(&messages).expect("the default, sent");
    assert!(sent <= Duration::from_millis(200), "{sent:?}");

    let (messages, _, _) = with(
        &channel,
        "/raw/EchoHeaders",
        Some(Deadline::Timeout(Duration::from_secs(30))),
    )
    .await;
    let sent = sent_timeout(&messages).expect("the call's own");
    assert!(sent > Duration::from_secs(25), "{sent:?}");
}

#[tokio::test]
async fn a_deadline_already_past_ends_the_call_without_reaching_the_server() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    for deadline in [
        Deadline::Timeout(Duration::ZERO),
        Deadline::Absolute(Instant::now() - Duration::from_millis(1)),
    ] {
        let (_, status, origin) = with(&channel, ECHO, Some(deadline)).await;
        assert_eq!(status.code, GrpcStatusCode::DeadlineExceeded, "{status}");
        assert_eq!(origin, HeadOrigin::NoResponse);
    }
    assert_eq!(server.connections(), 0, "the server was dialled");
}

/// A deadline far past what the header can state is still sent, at the largest it can.
#[tokio::test]
async fn a_deadline_past_what_the_header_can_state_is_sent_as_the_largest_it_can() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    let (messages, status, _) = with(
        &channel,
        "/raw/EchoHeaders",
        Some(Deadline::Timeout(Duration::from_secs(1_000_000_000_000))),
    )
    .await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(
        sent_timeout(&messages),
        Some(Duration::from_secs(99_999_999 * 3600))
    );
}

/// A deadline past what the clock can hold is no deadline, rather than a panic in the addition.
#[tokio::test]
async fn a_deadline_beyond_the_clock_is_none() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    let (messages, status, _) = with(
        &channel,
        "/raw/EchoHeaders",
        Some(Deadline::Timeout(Duration::MAX)),
    )
    .await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(sent_timeout(&messages), None);
}
