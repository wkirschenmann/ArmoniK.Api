//! Where the end of each attempt came from: the server's status, an HTTP error of a proxy, a
//! reset and its reason, a GOAWAY, a dial that failed, a stream that broke after its head, and the
//! engine's own refusals, and what its server said of a retry.
//!
//! The attempts are told by a hook that is the process's, so the tests hold one lock and run one
//! at a time.

mod common;

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use armonik_transport::grpc::{
    CallStartOptions, Cause, GrpcChannel, GrpcChannelConfig, GrpcStatusCode, MetadataValue, Origin,
    Pushback, RetryConfig,
};
use armonik_transport::hooks::{self, Attempt};
use armonik_transport::http2::TransportConfig;
use bytes::Bytes;
use common::echo::*;
use common::refuser::{Refusal, Refuser};
use http::{StatusCode, Uri};

static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// The attempts that ended while it lived, in order.
struct Recording {
    attempts: Arc<Mutex<Vec<Attempt>>>,
    _held: MutexGuard<'static, ()>,
}

impl Recording {
    fn start() -> Self {
        let held = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
        let attempts = Arc::new(Mutex::new(Vec::new()));
        let told = Arc::clone(&attempts);
        hooks::on_attempt(Some(Arc::new(move |attempt: &Attempt| {
            told.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(attempt.clone());
        })));
        Self {
            attempts,
            _held: held,
        }
    }

    fn attempts(&self) -> Vec<Attempt> {
        self.attempts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The origin and the code of each attempt.
    fn ends(&self) -> Vec<(Origin, GrpcStatusCode)> {
        self.attempts()
            .into_iter()
            .map(|attempt| (attempt.origin, attempt.code))
            .collect()
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        hooks::on_attempt(None);
    }
}

/// A channel that retries as `GrpcClient` does, within a few milliseconds.
fn retrying(endpoint: &str, attempts: u32) -> GrpcChannel {
    let mut config = GrpcChannelConfig::new(TransportConfig::new(
        Uri::try_from(endpoint).expect("an endpoint"),
    ));
    let mut retry = RetryConfig::default();
    retry.failures = vec![
        Cause::Status(GrpcStatusCode::Unavailable),
        Cause::Status(GrpcStatusCode::Aborted),
        Cause::Status(GrpcStatusCode::Unknown),
        Cause::Dial,
        Cause::Connection,
    ];
    retry.max_attempts = attempts;
    retry.initial_backoff = Duration::from_millis(5);
    retry.max_backoff = Duration::from_millis(20);
    config.retry = Some(retry);
    channel_with(config).expect("a channel")
}

fn with(mut options: CallStartOptions, name: &str, value: &str) -> CallStartOptions {
    options
        .metadata
        .append(name, MetadataValue::Ascii(value.to_owned()))
        .expect("a header");
    options
}

/// A flaky call that fails once with `code` and what `extra` adds.
fn flaky(key: &str, code: &str, extra: &[(&str, &str)]) -> CallStartOptions {
    let mut options = with(CallStartOptions::new(FLAKY), "x-flaky-key", key);
    options = with(options, "x-fail-times", "1");
    options = with(options, "x-fail-code", code);
    for (name, value) in extra {
        options = with(options, name, value);
    }
    options
}

async fn run(channel: &GrpcChannel, options: CallStartOptions) -> GrpcStatusCode {
    unary(channel, options, Bytes::from_static(b"x"))
        .await
        .2
        .code
}

#[tokio::test]
async fn the_status_a_server_gives_is_the_servers_whatever_its_code_or_its_place() {
    let recorded = Recording::start();
    let server = TestServer::start().await;
    let channel = retrying(&server.endpoint, 5);

    run(&channel, flaky("origin-unavailable", "14", &[])).await;
    run(&channel, flaky("origin-exhausted", "8", &[])).await;
    run(
        &channel,
        flaky("origin-after-head", "14", &[("x-fail-after-head", "1")]),
    )
    .await;

    assert_eq!(
        recorded.ends(),
        vec![
            (Origin::Server, GrpcStatusCode::Unavailable),
            (Origin::Server, GrpcStatusCode::Ok),
            (Origin::Server, GrpcStatusCode::ResourceExhausted),
            (Origin::Server, GrpcStatusCode::Unavailable),
        ]
    );
}

#[tokio::test]
async fn a_pushback_is_read_wherever_the_attempt_failed() {
    let recorded = Recording::start();
    let server = TestServer::start().await;
    let channel = retrying(&server.endpoint, 5);

    run(
        &channel,
        flaky("pushback-wait", "14", &[("x-pushback", "40")]),
    )
    .await;
    run(
        &channel,
        flaky("pushback-refused", "14", &[("x-pushback", "-1")]),
    )
    .await;
    run(&channel, CallStartOptions::new("/raw/PushbackAfterHead")).await;

    let pushbacks: Vec<_> = recorded
        .attempts()
        .into_iter()
        .map(|attempt| (attempt.code, attempt.pushback))
        .collect();
    assert_eq!(
        pushbacks,
        vec![
            (
                GrpcStatusCode::Unavailable,
                Pushback::After(Duration::from_millis(40))
            ),
            (GrpcStatusCode::Ok, Pushback::Unsaid),
            (GrpcStatusCode::Unavailable, Pushback::Refused),
            (
                GrpcStatusCode::Unavailable,
                Pushback::After(Duration::from_millis(700))
            ),
        ],
        "the last is a call whose head came: its trailers are read too"
    );
}

#[tokio::test]
async fn an_answer_with_no_status_is_the_proxys_by_its_http_status() {
    let recorded = Recording::start();
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    for status in [429, 502, 503, 504, 500, 408, 404, 401, 400] {
        let options = with(
            CallStartOptions::new("/raw/HttpError"),
            "x-http-status",
            &status.to_string(),
        );
        run(&channel, options).await;
    }
    run(&channel, CallStartOptions::new("/raw/NotGrpc")).await;
    // A status the peer states stands even behind an HTTP error, and is the server's.
    run(&channel, CallStartOptions::new("/raw/StatusBehindError")).await;

    let origins: Vec<_> = recorded
        .attempts()
        .into_iter()
        .map(|attempt| attempt.origin)
        .collect();
    let mut expected: Vec<_> = [429, 502, 503, 504, 500, 408, 404, 401, 400]
        .into_iter()
        .map(|status| Origin::Http(StatusCode::from_u16(status).expect("a status")))
        .collect();
    expected.push(Origin::Http(StatusCode::OK));
    expected.push(Origin::Server);
    assert_eq!(origins, expected);
}

#[tokio::test]
async fn a_reset_is_the_peers_with_its_reason_and_a_goaway_is_its_own_origin() {
    let recorded = Recording::start();

    for (refusal, expected) in [
        (
            Refusal::RefusedStream,
            vec![
                (
                    Origin::Reset(h2::Reason::REFUSED_STREAM),
                    GrpcStatusCode::Unavailable,
                ),
                (Origin::Server, GrpcStatusCode::Ok),
            ],
        ),
        (
            Refusal::GoAway,
            vec![
                (Origin::GoAway, GrpcStatusCode::Unavailable),
                // The connection closes under the resend, which hyper never sends.
                (Origin::Unsent, GrpcStatusCode::Unavailable),
                (Origin::Server, GrpcStatusCode::Ok),
            ],
        ),
        (
            Refusal::InternalError,
            vec![(
                Origin::Reset(h2::Reason::INTERNAL_ERROR),
                GrpcStatusCode::Internal,
            )],
        ),
    ] {
        let refuser = Refuser::start(refusal, 1).await;
        let channel = retrying(&refuser.endpoint, 5);
        run(&channel, CallStartOptions::new(ECHO)).await;

        assert_eq!(recorded.ends(), expected, "{refusal:?}");
        recorded.attempts.lock().expect("the record").clear();
    }
}

/// Nothing in the error says a GOAWAY came first: h2 gives the stream the connection's end, an
/// I/O error, and the engine has no word of the GOAWAY the session received. Such a stream is
/// told apart from a dead connection only by a session that reports the frame.
#[tokio::test]
async fn a_goaway_that_names_the_stream_as_processed_ends_it_as_the_connection_does() {
    let recorded = Recording::start();
    let refuser = Refuser::start(Refusal::GoAwayProcessed, 1).await;
    let channel = retrying(&refuser.endpoint, 1);

    run(&channel, CallStartOptions::new(ECHO)).await;

    assert_eq!(
        recorded.ends(),
        vec![(Origin::Connection, GrpcStatusCode::Unavailable)]
    );
}

#[tokio::test]
async fn a_dial_that_fails_is_the_dials() {
    let recorded = Recording::start();
    let (listener, endpoint) = loopback().await;
    drop(listener);
    let channel = retrying(&endpoint, 2);

    run(&channel, CallStartOptions::new(ECHO)).await;

    assert_eq!(
        recorded.ends(),
        vec![
            (Origin::Dial, GrpcStatusCode::Unavailable),
            (Origin::Dial, GrpcStatusCode::Unavailable)
        ]
    );
}

#[tokio::test]
async fn a_stream_that_breaks_after_its_head_is_not_a_reset_before_it() {
    let recorded = Recording::start();
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    run(&channel, CallStartOptions::new("/raw/PacedReset")).await;

    assert_eq!(
        recorded.ends(),
        vec![(Origin::Broke, GrpcStatusCode::Internal)]
    );
}

#[tokio::test]
async fn what_the_engine_refuses_itself_is_its_own() {
    let recorded = Recording::start();
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    // A message past the receive limit, and a status stated before a message.
    run(&channel, CallStartOptions::new("/raw/TooBig")).await;
    run(
        &channel,
        CallStartOptions::new("/raw/StatusInHeadThenMessage"),
    )
    .await;
    assert_eq!(
        recorded.ends(),
        vec![
            (Origin::Local, GrpcStatusCode::ResourceExhausted),
            (Origin::Local, GrpcStatusCode::Unknown),
        ]
    );

    // A header list past its limit ends the attempt before anything is sent, where the engine
    // adds its headers.
    let mut config = GrpcChannelConfig::new(TransportConfig::new(
        Uri::try_from(server.endpoint.as_str()).expect("an endpoint"),
    ));
    config.transport.http2.max_header_list_size = Some(8);
    let narrow = channel_with(config).expect("a channel");
    recorded.attempts.lock().expect("the record").clear();
    run(&narrow, CallStartOptions::new(ECHO)).await;
    assert_eq!(
        recorded.ends(),
        vec![(Origin::Local, GrpcStatusCode::ResourceExhausted)]
    );
}

/// An answer the peer ended without a status of its own, or ended in the middle of a message, is
/// no status of the server's: the engine or tonic made the status up.
#[tokio::test]
async fn a_status_made_up_of_an_answer_that_ended_badly_is_not_the_servers() {
    let recorded = Recording::start();
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    run(&channel, CallStartOptions::new("/raw/EndsMidMessage")).await;
    run(&channel, CallStartOptions::new("/raw/NoTrailers")).await;

    assert_eq!(
        recorded.ends(),
        vec![
            (Origin::Local, GrpcStatusCode::Internal),
            (Origin::Local, GrpcStatusCode::Unknown),
        ]
    );
}

/// A policy may name a code the engine also gives for its own refusals, and the engine's own
/// refusal is not the server's to try again.
#[tokio::test]
async fn an_attempt_the_engine_ended_is_not_tried_again_whatever_codes_the_policy_names() {
    let recorded = Recording::start();
    let server = TestServer::start().await;
    let mut config = GrpcChannelConfig::new(TransportConfig::new(
        Uri::try_from(server.endpoint.as_str()).expect("an endpoint"),
    ));
    config.transport.http2.max_header_list_size = Some(8);
    let mut retry = RetryConfig::default();
    retry.failures = vec![Cause::Status(GrpcStatusCode::ResourceExhausted)];
    retry.max_attempts = 3;
    retry.initial_backoff = Duration::from_millis(5);
    retry.max_backoff = Duration::from_millis(20);
    config.retry = Some(retry);
    let channel = channel_with(config).expect("a channel");

    let code = run(&channel, CallStartOptions::new(ECHO)).await;

    assert_eq!(code, GrpcStatusCode::ResourceExhausted);
    assert_eq!(
        recorded.ends(),
        vec![(Origin::Local, GrpcStatusCode::ResourceExhausted)],
        "one attempt, and no retry"
    );
}

/// The call's own refusal of a message stops it before an attempt goes out: the call is ended
/// while it asks for its first turn.
#[tokio::test]
async fn a_message_past_the_send_limit_stops_the_call_before_an_attempt() {
    let recorded = Recording::start();
    let server = TestServer::start().await;
    let mut config = GrpcChannelConfig::new(TransportConfig::new(
        Uri::try_from(server.endpoint.as_str()).expect("an endpoint"),
    ));
    config.max_send_message_size = Some(1);
    let channel = channel_with(config).expect("a channel");

    let (_, _, status) = unary(
        &channel,
        CallStartOptions::new(ECHO),
        Bytes::from_static(b"xx"),
    )
    .await;

    assert_eq!(status.code, GrpcStatusCode::ResourceExhausted);
    assert_eq!(recorded.ends(), vec![]);
}
