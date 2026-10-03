//! A call sent again after it failed, as gRFC A6 has it: what is retried, how many times, after
//! how long, and what commits a call so it is not.

mod common;

use std::time::{Duration, Instant};

use armonik_transport::grpc::{
    CallStartOptions, Deadline, GrpcChannel, GrpcChannelConfig, GrpcStatus, GrpcStatusCode,
    MetadataValue, RecvResult, RetryConfig,
};
use armonik_transport::http2::TransportConfig;
use bytes::Bytes;
use common::echo::*;
use common::refuser::{Refusal, Refuser};
use http::Uri;

fn retrying(endpoint: &str, change: impl FnOnce(&mut RetryConfig)) -> GrpcChannel {
    let mut config = GrpcChannelConfig::new(TransportConfig::new(
        Uri::try_from(endpoint).expect("an endpoint"),
    ));
    let mut retry = RetryConfig::default();
    retry.initial_backoff = Duration::from_millis(10);
    retry.max_backoff = Duration::from_millis(50);
    change(&mut retry);
    config.retry = Some(retry);
    channel_with(config).expect("a channel")
}

/// The options of a flaky call under a key of its own, failing `times` times with what `extra`
/// adds.
fn flaky_options(key: &str, times: usize, extra: &[(&str, &str)]) -> CallStartOptions {
    flaky_call(FLAKY, key, times, extra)
}

/// [`flaky_options`], for `method`.
fn flaky_call(method: &str, key: &str, times: usize, extra: &[(&str, &str)]) -> CallStartOptions {
    let mut options = CallStartOptions::new(method);
    let mut set = |name: &str, value: String| {
        options
            .metadata
            .append(name, MetadataValue::Ascii(value))
            .expect("a header");
    };
    set("x-flaky-key", key.to_owned());
    set("x-fail-times", times.to_string());
    for (name, value) in extra {
        set(name, (*value).to_owned());
    }
    options
}

async fn call(
    channel: &GrpcChannel,
    options: CallStartOptions,
    message: &'static [u8],
) -> (Vec<Bytes>, GrpcStatus) {
    let (_, messages, status) = unary(channel, options, Bytes::from_static(message)).await;
    (messages, status)
}

#[tokio::test]
async fn a_call_that_fails_unavailable_succeeds_on_its_second_attempt() {
    let server = TestServer::start().await;
    let channel = retrying(&server.endpoint, |_| {});

    let (messages, status) = call(&channel, flaky_options("second", 1, &[]), b"hello").await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(messages, vec![Bytes::from_static(b"hello")]);
    assert_eq!(
        flaky_seen("second"),
        vec![None, Some("1".to_owned())],
        "the second attempt says one went before it"
    );
}

#[tokio::test]
async fn a_code_the_policy_does_not_name_is_not_retried() {
    let server = TestServer::start().await;
    let channel = retrying(&server.endpoint, |_| {});

    let options = flaky_options("denied", 1, &[("x-fail-code", "7")]);
    let (_, status) = call(&channel, options, b"x").await;
    assert_eq!(status.code, GrpcStatusCode::PermissionDenied, "{status}");
    assert_eq!(flaky_seen("denied").len(), 1);
}

#[tokio::test]
async fn the_attempts_stop_at_the_policys_maximum() {
    let server = TestServer::start().await;
    let channel = retrying(&server.endpoint, |retry| retry.max_attempts = 3);

    let (_, status) = call(&channel, flaky_options("exhausted", 10, &[]), b"x").await;
    assert_eq!(status.code, GrpcStatusCode::Unavailable, "{status}");
    assert_eq!(
        flaky_seen("exhausted"),
        vec![None, Some("1".to_owned()), Some("2".to_owned())]
    );
}

#[tokio::test]
async fn a_call_whose_head_reached_the_reader_is_not_retried() {
    let server = TestServer::start().await;
    let channel = retrying(&server.endpoint, |_| {});

    let options = flaky_options("headed", 1, &[("x-fail-after-head", "1")]);
    let (_, status) = call(&channel, options, b"x").await;
    assert_eq!(status.code, GrpcStatusCode::Unavailable, "{status}");
    assert_eq!(flaky_seen("headed").len(), 1);
}

#[tokio::test]
async fn a_call_past_its_replay_ceiling_or_the_channels_is_committed() {
    let server = TestServer::start().await;
    for (key, call_bytes, channel_bytes) in [("ceiling", 2, 1 << 20), ("channel-total", 1 << 20, 2)]
    {
        let channel = retrying(&server.endpoint, |retry| {
            retry.call_replay_bytes = call_bytes;
            retry.channel_replay_bytes = channel_bytes;
        });
        let (_, status) = call(&channel, flaky_options(key, 1, &[]), b"hello").await;
        assert_eq!(status.code, GrpcStatusCode::Unavailable, "{key}: {status}");
        assert_eq!(flaky_seen(key).len(), 1, "{key}");
    }
}

#[tokio::test]
async fn the_servers_pushback_sets_the_wait_or_refuses_the_retry() {
    let server = TestServer::start().await;
    let channel = retrying(&server.endpoint, |_| {});

    let started = Instant::now();
    let options = flaky_options("pushed", 1, &[("x-pushback", "300")]);
    let (_, status) = call(&channel, options, b"x").await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert!(
        started.elapsed() >= Duration::from_millis(300),
        "{:?}",
        started.elapsed()
    );

    let options = flaky_options("refused", 1, &[("x-pushback", "-1")]);
    let (_, status) = call(&channel, options, b"x").await;
    assert_eq!(status.code, GrpcStatusCode::Unavailable, "{status}");
    assert_eq!(flaky_seen("refused").len(), 1);
}

/// The server asks for a wait the deadline would cut short: the call ends with what it failed
/// with, at once, and not with a DEADLINE_EXCEEDED the wait would earn it.
#[tokio::test]
async fn a_wait_past_the_deadline_ends_the_call_with_its_failure() {
    let server = TestServer::start().await;
    let channel = retrying(&server.endpoint, |_| {});

    let mut options = flaky_options("deadline", 1, &[("x-pushback", "30000")]);
    options.deadline = Some(Deadline::Timeout(Duration::from_secs(2)));
    let started = Instant::now();
    let (_, status) = call(&channel, options, b"x").await;
    assert_eq!(status.code, GrpcStatusCode::Unavailable, "{status}");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(flaky_seen("deadline").len(), 1);
}

/// Sends `messages`, half-closes, and reads the call to its end.
async fn stream(
    channel: &GrpcChannel,
    options: CallStartOptions,
    messages: &[&'static str],
) -> (Vec<Bytes>, GrpcStatus) {
    let (mut send, mut recv, _control) = channel.start_call(options).expect("a call").split();
    for message in messages {
        send.send_message(Bytes::from_static(message.as_bytes()))
            .await
            .expect("sent");
    }
    send.end_send().await.expect("the half-close");
    let (_, messages, status) = read_to_terminal(&mut recv).await;
    (messages, status)
}

/// A client stream that fails once its server has read its first message is sent again whole:
/// what was kept, then what the host sends after.
#[tokio::test]
async fn a_client_stream_within_its_ceiling_is_sent_again_whole() {
    let server = TestServer::start().await;
    let channel = retrying(&server.endpoint, |_| {});

    let options = flaky_call(
        FLAKY_COLLECT,
        "client-stream",
        1,
        &[("x-fail-after-messages", "1")],
    );
    let (messages, status) = stream(&channel, options, &["one", "two", "three"]).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(messages, vec![Bytes::from_static(b"3:one,two,three")]);
    assert_eq!(
        flaky_seen("client-stream"),
        vec![None, Some("1".to_owned())]
    );
}

/// A bidi stream that fails before answering anything is sent again, and the attempt that
/// succeeds answers what the first one was sent.
#[tokio::test]
async fn a_bidi_stream_nothing_answered_is_sent_again() {
    let server = TestServer::start().await;
    let channel = retrying(&server.endpoint, |_| {});

    let options = flaky_call(
        FLAKY_CHAT,
        "bidi-unanswered",
        1,
        &[("x-fail-after-messages", "1")],
    );
    let (mut send, mut recv, _control) = channel.start_call(options).expect("a call").split();
    for text in ["one", "two"] {
        send.send_message(Bytes::from_static(text.as_bytes()))
            .await
            .expect("sent");
        // Bounded, as an attempt that replays nothing leaves the server waiting for a message.
        let answer = tokio::time::timeout(Duration::from_secs(10), recv.next_message())
            .await
            .expect("an answer in time");
        match answer.expect("an answer") {
            RecvResult::Message(answered) => {
                assert_eq!(answered.data, Bytes::from_static(text.as_bytes()))
            }
            other => panic!("{other:?}"),
        }
    }
    send.end_send().await.expect("the half-close");
    let (_, messages, status) = read_to_terminal(&mut recv).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert!(messages.is_empty(), "{messages:?}");
    assert_eq!(
        flaky_seen("bidi-unanswered"),
        vec![None, Some("1".to_owned())]
    );
}

/// A bidi stream whose server answered before failing is committed: its reader saw the answer,
/// and the failure ends the call.
#[tokio::test]
async fn a_bidi_stream_answered_is_not_sent_again() {
    let server = TestServer::start().await;
    let channel = retrying(&server.endpoint, |_| {});

    let options = flaky_call(
        FLAKY_CHAT,
        "bidi-answered",
        1,
        &[("x-fail-after-messages", "1"), ("x-answer-first", "1")],
    );
    let (messages, status) = stream(&channel, options, &["one"]).await;
    assert_eq!(status.code, GrpcStatusCode::Unavailable, "{status}");
    assert_eq!(messages, vec![Bytes::from_static(b"one")]);
    assert_eq!(flaky_seen("bidi-answered").len(), 1);
}

/// A stream of either kind past its replay ceiling is committed, and its failure ends it.
#[tokio::test]
async fn a_stream_past_its_ceiling_is_not_sent_again() {
    let server = TestServer::start().await;
    let channel = retrying(&server.endpoint, |retry| retry.call_replay_bytes = 4);

    for (method, key) in [
        (FLAKY_COLLECT, "client-stream-past"),
        (FLAKY_CHAT, "bidi-past"),
    ] {
        let options = flaky_call(method, key, 1, &[("x-fail-after-messages", "2")]);
        let (_, status) = stream(&channel, options, &["one", "two"]).await;
        assert_eq!(status.code, GrpcStatusCode::Unavailable, "{key}: {status}");
        assert_eq!(flaky_seen(key).len(), 1, "{key}");
    }
}

/// A stream the peer's HTTP/2 layer turned away goes again at once, as gRFC A6's transparent
/// retry has it: under a policy of one attempt, which retries nothing it counts, and with no
/// `grpc-previous-rpc-attempts`, since the first attempt is not one.
#[tokio::test]
async fn a_stream_the_peer_never_processed_goes_again_counting_no_attempt() {
    for refusal in [Refusal::RefusedStream, Refusal::GoAway] {
        let refuser = Refuser::start(refusal, 1).await;
        let channel = retrying(&refuser.endpoint, |retry| {
            retry.max_attempts = 1;
            retry.initial_backoff = Duration::from_secs(30);
            retry.max_backoff = Duration::from_secs(30);
        });

        let (messages, status) = call(&channel, CallStartOptions::new(ECHO), b"hello").await;
        assert_eq!(status.code, GrpcStatusCode::Ok, "{refusal:?}: {status}");
        assert_eq!(messages, vec![Bytes::from_static(b"hello")], "{refusal:?}");
        assert_eq!(refuser.seen(), vec![None, None], "{refusal:?}");
    }
}

/// A GOAWAY that names the stream as processed leaves it to end as the connection does, and the
/// call is the policy's, which retries nothing at one attempt.
#[tokio::test]
async fn a_stream_a_goaway_names_as_processed_is_not_sent_again() {
    let refuser = Refuser::start(Refusal::GoAwayProcessed, 1).await;
    let channel = retrying(&refuser.endpoint, |retry| retry.max_attempts = 1);
    let (_, status) = call(&channel, CallStartOptions::new(ECHO), b"hello").await;
    assert_eq!(status.code, GrpcStatusCode::Unavailable, "{status}");
    assert_eq!(refuser.seen().len(), 1);
}

/// A reset for any other reason does not say the peer left the stream unprocessed, and the call
/// is the policy's.
#[tokio::test]
async fn a_stream_reset_for_another_reason_is_not_sent_again() {
    let refuser = Refuser::start(Refusal::InternalError, 1).await;
    let channel = retrying(&refuser.endpoint, |retry| retry.max_attempts = 1);
    let (_, status) = call(&channel, CallStartOptions::new(ECHO), b"hello").await;
    assert_eq!(status.code, GrpcStatusCode::Internal, "{status}");
    assert_eq!(refuser.seen().len(), 1);
}

/// Once a call: a second refusal is the policy's to retry, after its backoff and as an attempt.
#[tokio::test]
async fn a_second_refusal_meets_the_policy() {
    let refuser = Refuser::start(Refusal::RefusedStream, 2).await;
    let channel = retrying(&refuser.endpoint, |retry| retry.max_attempts = 1);
    let (_, status) = call(&channel, CallStartOptions::new(ECHO), b"x").await;
    assert_eq!(status.code, GrpcStatusCode::Unavailable, "{status}");
    assert_eq!(refuser.seen().len(), 2);

    let refuser = Refuser::start(Refusal::RefusedStream, 2).await;
    let channel = retrying(&refuser.endpoint, |retry| retry.max_attempts = 2);
    let (_, status) = call(&channel, CallStartOptions::new(ECHO), b"x").await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(refuser.seen(), vec![None, None, Some("1".to_owned())]);
}

/// A call that did not keep all it sent is not sent again, even unprocessed: what it sent could not
/// be sent again whole. Past its ceiling here, and with no policy, which keeps nothing.
#[tokio::test]
async fn a_call_refused_without_all_it_sent_kept_is_not_sent_again() {
    let past_ceiling = |endpoint: &str| retrying(endpoint, |retry| retry.call_replay_bytes = 2);
    let no_policy = |endpoint: &str| channel(endpoint);
    for (case, open) in [
        (
            "past its ceiling",
            &past_ceiling as &dyn Fn(&str) -> GrpcChannel,
        ),
        ("no policy", &no_policy),
    ] {
        let refuser = Refuser::start(Refusal::RefusedStream, 1).await;
        let (_, status) = call(
            &open(&refuser.endpoint),
            CallStartOptions::new(ECHO),
            b"hello",
        )
        .await;
        assert_eq!(status.code, GrpcStatusCode::Unavailable, "{case}: {status}");
        assert_eq!(refuser.seen().len(), 1, "{case}");
    }
}

#[tokio::test]
async fn a_channel_with_no_policy_does_not_retry() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    let (_, status) = call(&channel, flaky_options("unretried", 1, &[]), b"x").await;
    assert_eq!(status.code, GrpcStatusCode::Unavailable, "{status}");
    assert_eq!(flaky_seen("unretried").len(), 1);
}
