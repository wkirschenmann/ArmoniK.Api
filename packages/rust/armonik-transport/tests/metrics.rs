//! What the engine counts as events happen: calls from their start to their end, messages and
//! bytes, retries by their failure, resends, dials and the reasons a session closes, resets, and the
//! gauges of the estimate.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use armonik_transport::grpc::{
    AdaptiveConfig, CallStartOptions, Cause, Charge, CompressionBudget, Encoding, FramedMessage,
    GrpcChannel, GrpcChannelConfig, GrpcStatus, GrpcStatusCode, MetadataValue, RecvResult,
    ReplayConfig, ResponseHead, ResponseSink, RetryConfig,
};
use armonik_transport::http2::TransportConfig;
use armonik_transport::metrics::{
    CloseReason, Metrics, Stats, CLOSE_SLOTS, RESET_SLOTS, RETRY_DIAL, RETRY_PUSHBACK,
    RETRY_RESET_AT, STATUS_SLOTS,
};
use bytes::Bytes;
use common::echo::*;
use common::refuser::{Refusal, Refuser};
use http::Uri;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

static KEYS: AtomicUsize = AtomicUsize::new(0);

fn config(endpoint: &str) -> GrpcChannelConfig {
    GrpcChannelConfig::new(TransportConfig::new(
        Uri::try_from(endpoint).expect("an endpoint"),
    ))
}

fn retrying(endpoint: &str, failures: Vec<Cause>, attempts: u32) -> GrpcChannel {
    let mut config = config(endpoint);
    let mut retry = RetryConfig::default();
    retry.failures = failures;
    retry.max_attempts = attempts;
    retry.initial_backoff = Duration::from_millis(2);
    retry.max_backoff = Duration::from_millis(5);
    config.retry = Some(retry);
    channel_with(config).expect("a channel")
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

async fn echo(channel: &GrpcChannel, message: &'static [u8]) {
    let (_, _, status) = unary(
        channel,
        CallStartOptions::new(ECHO),
        Bytes::from_static(message),
    )
    .await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
}

fn flaky(code: &str, times: usize, extra: &[(&str, &str)]) -> CallStartOptions {
    let key = format!("metrics-{}", KEYS.fetch_add(1, Ordering::SeqCst));
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
    options
}

fn closed(stats: &Stats, reason: CloseReason) -> u64 {
    stats.connections_closed[reason as usize]
}

#[test]
fn a_build_with_the_feature_says_it_counts() {
    let metrics = Metrics::new();
    let stats = metrics.stats();
    assert!(stats.counting);
    assert_eq!(stats.calls_started, 0);
    assert_eq!(stats.calls_ended, [0; STATUS_SLOTS]);
    assert_eq!(stats.connections_closed, [0; CLOSE_SLOTS]);
    assert_eq!(stats.streams_reset, [0; RESET_SLOTS]);
}

#[tokio::test]
async fn a_call_is_counted_from_its_start_to_its_end() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);
    assert_eq!(channel.stats().calls_started, 0);

    echo(&channel, b"hello").await;

    let stats = channel.stats();
    assert_eq!(stats.calls_started, 1);
    assert_eq!(stats.ended_with(GrpcStatusCode::Ok), 1);
    assert_eq!(stats.calls_ended.iter().sum::<u64>(), 1);
    assert_eq!((stats.messages_sent, stats.messages_received), (1, 1));
    assert_eq!((stats.message_bytes_raw, stats.message_bytes_sent), (5, 5));
    assert!(stats.wire_bytes_sent > 0 && stats.wire_bytes_received > 0);
    assert_eq!(
        (stats.dials_tried, stats.dials_succeeded, stats.dials_failed),
        (1, 1, 0)
    );
}

#[tokio::test]
async fn a_call_is_counted_by_the_status_it_ends_with() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    unary(&channel, CallStartOptions::new(FAIL), Bytes::new()).await;
    unary(
        &channel,
        CallStartOptions::new("/nowhere.Nothing/Nothing"),
        Bytes::new(),
    )
    .await;
    echo(&channel, b"x").await;

    let stats = channel.stats();
    assert_eq!(stats.calls_started, 3);
    assert_eq!(stats.ended_with(GrpcStatusCode::PermissionDenied), 1);
    assert_eq!(stats.ended_with(GrpcStatusCode::Unimplemented), 1);
    assert_eq!(stats.ended_with(GrpcStatusCode::Ok), 1);
}

/// A stream read twice while it is open: what it counted so far is there each time.
#[tokio::test]
async fn a_stream_that_is_open_is_counted_as_it_goes() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    let (mut send, mut recv, _control) = channel
        .start_call(CallStartOptions::new(CHAT))
        .expect("the call starts")
        .split();

    send.send_message(Bytes::from_static(b"one"))
        .await
        .expect("sent");
    match recv.next_message().await.expect("an answer") {
        RecvResult::Message(_) => {}
        other => panic!("{other:?}"),
    }
    let first = channel.stats();
    assert_eq!(first.calls_started, 1);
    assert_eq!(first.calls_ended, [0; STATUS_SLOTS], "the call is open");
    assert_eq!((first.messages_sent, first.messages_received), (1, 1));

    send.send_message(Bytes::from_static(b"two"))
        .await
        .expect("sent");
    match recv.next_message().await.expect("an answer") {
        RecvResult::Message(_) => {}
        other => panic!("{other:?}"),
    }
    let second = channel.stats();
    assert_eq!((second.messages_sent, second.messages_received), (2, 2));
    assert_eq!(second.message_bytes_raw, 6);
    assert!(second.wire_bytes_sent > first.wire_bytes_sent);
    assert!(second.wire_bytes_received > first.wire_bytes_received);
    assert_eq!(second.calls_ended, [0; STATUS_SLOTS], "still open");

    send.end_send().await.expect("the half-close");
    match recv.next_message().await.expect("a terminal") {
        RecvResult::End(status) => assert_eq!(status.code, GrpcStatusCode::Ok),
        other => panic!("{other:?}"),
    }
    assert_eq!(channel.stats().ended_with(GrpcStatusCode::Ok), 1);
}

/// The host's delivery of the last messages of a call is counted by the call's own task, which
/// runs after the driver has ended the call.
#[tokio::test]
async fn a_count_made_after_the_call_ended_is_in_the_stats() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    let (mut send, mut recv, control) = channel
        .start_call(CallStartOptions::new(CHAT))
        .expect("the call starts")
        .split();
    send.send_message(Bytes::from_static(b"one"))
        .await
        .expect("sent");
    send.end_send().await.expect("the half-close");
    loop {
        match recv.next_message().await.expect("an answer") {
            RecvResult::Message(_) => {}
            RecvResult::End(status) => {
                assert_eq!(status.code, GrpcStatusCode::Ok);
                break;
            }
        }
    }
    assert_eq!(channel.stats().ended_with(GrpcStatusCode::Ok), 1);
    assert_eq!(channel.stats().host_window_waits, 0);

    control.count_window_wait();
    control.count_window_wait();
    assert_eq!(channel.stats().host_window_waits, 2);

    drop((recv, control));
    let stats = channel.stats();
    assert_eq!(stats.host_window_waits, 2, "the call left its counts");
    assert_eq!((stats.messages_sent, stats.messages_received), (1, 1));
    assert_eq!(stats.ended_with(GrpcStatusCode::Ok), 1);
}

#[tokio::test]
async fn a_message_is_counted_before_and_after_its_compression() {
    let server = TestServer::start().await;
    let mut config = config(&server.endpoint);
    config.send_encoding = Some(Encoding::Gzip);
    let channel = channel_with(config).expect("a channel");

    let message = Bytes::from(vec![b'a'; 8 * 1024]);
    let (_, _, status) = unary(&channel, CallStartOptions::new(ECHO_COMPRESSED), message).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");

    let stats = channel.stats();
    assert_eq!(stats.message_bytes_raw, 8 * 1024);
    assert!(
        stats.message_bytes_sent < 1024,
        "{} bytes of one letter compress",
        stats.message_bytes_sent
    );
}

/// A budget with no room for any compressed copy.
#[derive(Debug)]
struct NoRoom;

impl CompressionBudget for NoRoom {
    fn charge(&self, _: usize) -> Option<Charge> {
        None
    }
}

/// A message whose compressed copy the memory ceiling has no room for goes out as written, and is
/// counted as sent at the length it went out with.
#[tokio::test]
async fn a_message_sent_uncompressed_for_want_of_room_is_counted_as_sent_whole() {
    let server = TestServer::start().await;
    let mut config = config(&server.endpoint);
    config.send_encoding = Some(Encoding::Gzip);
    let channel = channel_with(config).expect("a channel");
    let refusing = |path: &str| {
        let mut options = CallStartOptions::new(path);
        options.compression_budget = Some(Arc::new(NoRoom));
        options
    };

    // The one request of a unary call.
    let message = Bytes::from(vec![b'a'; 8 * 1024]);
    let (_, _, status) = unary(&channel, refusing(ECHO_COMPRESSED), message).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    let one = channel.stats();
    assert_eq!(
        (one.message_bytes_raw, one.message_bytes_sent),
        (8 * 1024, 8 * 1024)
    );

    // A message of a stream, which the server reads whole.
    let (mut send, mut recv, _control) = channel
        .start_call(refusing(FRAMES))
        .expect("the call starts")
        .split();
    send.send_message(Bytes::from(vec![b'a'; 8 * 1024]))
        .await
        .expect("sent");
    send.end_send().await.expect("the half-close");
    loop {
        match recv.next_message().await.expect("an answer") {
            RecvResult::Message(_) => {}
            RecvResult::End(status) => {
                assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
                break;
            }
        }
    }
    let both = channel.stats();
    assert_eq!(
        (both.message_bytes_raw, both.message_bytes_sent),
        (16 * 1024, 16 * 1024),
        "the gain is none when nothing could be copied"
    );
}

/// A sink that takes what a call answers and says nothing of it.
struct Quiet(Option<tokio::sync::oneshot::Sender<GrpcStatus>>);

impl ResponseSink for Quiet {
    async fn head(&mut self, _: ResponseHead) -> Result<(), GrpcStatus> {
        Ok(())
    }

    async fn message(&mut self, _: Bytes) -> Result<(), GrpcStatus> {
        Ok(())
    }

    fn flush(&mut self) {}

    async fn end(mut self, status: GrpcStatus, _: Option<ResponseHead>) {
        if let Some(done) = self.0.take() {
            let _ = done.send(status);
        }
    }
}

/// The one request of a unary call is a message too, counted once the call has it.
#[tokio::test]
async fn the_one_request_of_a_unary_call_is_counted_as_a_message() {
    let server = TestServer::start().await;
    for (encoding, compresses) in [(None, false), (Some(Encoding::Gzip), true)] {
        let mut config = config(&server.endpoint);
        config.send_encoding = encoding;
        let channel = channel_with(config).expect("a channel");

        let (request, _control, driver) = channel
            .prepare_one_request_call(CallStartOptions::new(ECHO_COMPRESSED))
            .expect("an open channel");
        let message = vec![b'a'; 8 * 1024];
        assert!(request.give(|| FramedMessage::copy_of(&message).expect("a message")));
        let (done, heard) = tokio::sync::oneshot::channel();
        driver.drive(Quiet(Some(done))).await;
        assert_eq!(heard.await.expect("an end").code, GrpcStatusCode::Ok);

        let stats = channel.stats();
        assert_eq!(stats.messages_sent, 1);
        assert_eq!(stats.message_bytes_raw, 8 * 1024);
        assert_eq!(stats.message_bytes_sent < 1024, compresses, "{stats:?}");
        assert_eq!(stats.messages_received, 1);
    }
}

#[tokio::test]
async fn a_retry_is_counted_by_the_failure_that_was_retried() {
    let server = TestServer::start().await;
    let channel = retrying(
        &server.endpoint,
        vec![Cause::Status(GrpcStatusCode::Unavailable)],
        5,
    );

    let (_, _, status) = unary(&channel, flaky("14", 2, &[]), Bytes::from_static(b"x")).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");

    let stats = channel.stats();
    let unavailable = GrpcStatusCode::Unavailable as usize - 1;
    assert_eq!(stats.retries[unavailable], 2);
    assert_eq!(stats.retries.iter().sum::<u64>(), 2);
    assert_eq!(stats.calls_started, 1, "a retry is not another call");
    assert_eq!(stats.messages_sent, 1, "a message is counted once");
    assert_eq!(stats.resends, 0);
}

#[tokio::test]
async fn a_pushback_alone_names_a_retry() {
    let server = TestServer::start().await;
    let channel = retrying(&server.endpoint, vec![Cause::Pushback], 5);

    let options = flaky("2", 1, &[("x-pushback", "1")]);
    let (_, _, status) = unary(&channel, options, Bytes::from_static(b"x")).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");

    let stats = channel.stats();
    assert_eq!(stats.retries[RETRY_PUSHBACK], 1);
    assert_eq!(stats.retries.iter().sum::<u64>(), 1);
}

#[tokio::test]
async fn a_dial_that_failed_is_a_retry_of_the_dial() {
    let endpoint = closed_port().await;
    let channel = retrying(&endpoint, vec![Cause::Dial], 3);

    let (_, _, status) = unary(&channel, CallStartOptions::new(ECHO), Bytes::new()).await;
    assert_eq!(status.code, GrpcStatusCode::Unavailable);

    let stats = channel.stats();
    assert_eq!(stats.retries[RETRY_DIAL], 2);
    assert_eq!(stats.dials_tried, 3);
    assert_eq!((stats.dials_succeeded, stats.dials_failed), (0, 3));
}

/// The peer never processed the first stream, so it is sent again at once, and that is no retry.
#[tokio::test]
async fn a_request_the_peer_never_processed_is_a_resend_and_not_a_retry() {
    let refuser = Refuser::start(Refusal::RefusedStream, 1).await;
    let channel = retrying(&refuser.endpoint, vec![Cause::Dial], 3);

    let (_, _, status) = unary(
        &channel,
        CallStartOptions::new(ECHO),
        Bytes::from_static(b"x"),
    )
    .await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");

    let stats = channel.stats();
    assert_eq!(stats.resends, 1);
    assert_eq!(stats.retries.iter().sum::<u64>(), 0);
    assert_eq!(stats.streams_reset[7], 1, "REFUSED_STREAM is code 7");
}

/// A reset the policy retries is counted by its code, in the slot of the reset origin.
#[tokio::test]
async fn a_reset_is_counted_by_its_code_and_retried_by_it() {
    let refuser = Refuser::start(Refusal::InternalError, 1).await;
    let channel = retrying(&refuser.endpoint, vec![Cause::Reset(2)], 3);

    let (_, _, status) = unary(
        &channel,
        CallStartOptions::new(ECHO),
        Bytes::from_static(b"x"),
    )
    .await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");

    let stats = channel.stats();
    assert_eq!(stats.streams_reset[2], 1);
    assert_eq!(stats.retries[RETRY_RESET_AT + 2], 1);
    assert_eq!(stats.retries.iter().sum::<u64>(), 1);
}

#[tokio::test]
async fn a_call_that_outgrows_the_replay_ceiling_is_counted_once() {
    let server = TestServer::start().await;
    let mut config = config(&server.endpoint);
    let mut replay = ReplayConfig::default();
    replay.call_bytes = 8;
    config.replay = replay;
    let channel = channel_with(config).expect("a channel");

    // Collected: no head reaches the caller before the end, which would commit the call.
    let (mut send, mut recv, _control) = channel
        .start_call(CallStartOptions::new(COLLECT))
        .expect("the call starts")
        .split();
    for _ in 0..3 {
        send.send_message(Bytes::from_static(b"12345"))
            .await
            .expect("sent");
    }
    send.end_send().await.expect("the half-close");
    let _ = read_to_terminal(&mut recv).await;

    assert_eq!(channel.stats().calls_not_replayable, 1);
}

#[tokio::test]
async fn a_session_that_idles_out_is_closed_for_that_reason() {
    let server = TestServer::start().await;
    let mut config = config(&server.endpoint);
    config.transport.http2.idle_timeout = Some(Duration::from_millis(100));
    let channel = channel_with(config).expect("a channel");

    echo(&channel, b"x").await;
    until("the idle session closed", || {
        closed(&channel.stats(), CloseReason::IdleTimeout) == 1
    })
    .await;

    echo(&channel, b"x").await;
    assert_eq!(
        channel.stats().dials_succeeded,
        2,
        "the next call dialled again"
    );
}

#[tokio::test]
async fn a_session_of_a_channel_that_closes_is_closed_for_that_reason() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    echo(&channel, b"x").await;
    channel.close();
    until("the session closed", || {
        closed(&channel.stats(), CloseReason::LocalClose) == 1
    })
    .await;
}

#[tokio::test]
async fn a_session_the_peer_sent_a_goaway_on_is_closed_for_that_reason() {
    let server = TestServer::closing_after(1).await;
    let channel = channel(&server.endpoint);

    echo(&channel, b"x").await;
    until("the session closed", || {
        closed(&channel.stats(), CloseReason::GoAway) == 1
    })
    .await;
    assert_eq!(channel.stats().connections_closed.iter().sum::<u64>(), 1);
}

/// A proxy in front of `upstream` that does what its test says with the connection.
#[derive(Clone, Copy)]
enum Fate {
    /// Passes bytes both ways until the client has sent its first request, then goes silent: it
    /// reads what the client sends and answers nothing, not even a PING.
    Silent,
    /// Closes the connection once the client has sent its first request, cleanly.
    Closes,
    /// Resets the connection once the client has sent its first request, which its own function
    /// does.
    Resets,
}

async fn fated(upstream: &str, fate: Fate) -> String {
    let upstream = upstream
        .strip_prefix("http://")
        .expect("a plain endpoint")
        .to_owned();
    let (listener, endpoint) = loopback().await;
    tokio::spawn(async move {
        while let Ok((client, _)) = listener.accept().await {
            let server = TcpStream::connect(&upstream).await.expect("the upstream");
            client.set_nodelay(true).expect("no delay");
            server.set_nodelay(true).expect("no delay");
            if matches!(fate, Fate::Resets) {
                // Closed with a reset, as a peer that died would: with its two halves kept
                // together, since a half dropped alone sends a FIN first.
                client
                    .set_linger(Some(Duration::ZERO))
                    .expect("a linger of zero");
                tokio::spawn(reset_after_the_first_request(client, server));
                continue;
            }
            tokio::spawn(async move {
                let (mut from_client, mut to_client) = client.into_split();
                let (mut from_server, mut to_server) = server.into_split();
                let go = std::sync::Arc::new(tokio::sync::Notify::new());
                let stop = go.clone();
                tokio::spawn(async move {
                    let mut buffer = vec![0; 16 * 1024];
                    loop {
                        tokio::select! {
                            read = from_server.read(&mut buffer) => match read {
                                Ok(0) | Err(_) => return,
                                Ok(n) => if to_client.write_all(&buffer[..n]).await.is_err() { return },
                            },
                            _ = stop.notified() => {
                                // The client is told nothing more.
                                match fate {
                                    Fate::Closes => { let _ = to_client.shutdown().await; }
                                    Fate::Silent | Fate::Resets => std::future::pending::<()>().await,
                                }
                                return;
                            }
                        }
                    }
                });
                let mut buffer = vec![0; 16 * 1024];
                let mut total = 0;
                loop {
                    match from_client.read(&mut buffer).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => {
                            total += n;
                            if to_server.write_all(&buffer[..n]).await.is_err() {
                                return;
                            }
                            // The preface, the settings and the first request's frames.
                            if total > 150 {
                                go.notify_one();
                                if matches!(fate, Fate::Silent) {
                                    // Read and drop what the client sends from now on.
                                    while from_client.read(&mut buffer).await.is_ok_and(|n| n > 0) {
                                    }
                                    return;
                                }
                            }
                        }
                    }
                }
            });
        }
    });
    endpoint
}

async fn reset_after_the_first_request(mut client: TcpStream, mut server: TcpStream) {
    let mut from_client = vec![0; 16 * 1024];
    let mut from_server = vec![0; 16 * 1024];
    let mut total = 0;
    loop {
        tokio::select! {
            read = client.read(&mut from_client) => match read {
                Ok(0) | Err(_) => return,
                Ok(n) => {
                    total += n;
                    if server.write_all(&from_client[..n]).await.is_err() {
                        return;
                    }
                    // The preface, the settings and the first request's frames.
                    if total > 150 {
                        return;
                    }
                }
            },
            read = server.read(&mut from_server) => match read {
                Ok(0) | Err(_) => return,
                Ok(n) => if client.write_all(&from_server[..n]).await.is_err() { return },
            },
        }
    }
}

#[tokio::test]
async fn a_session_whose_keepalive_goes_unanswered_is_closed_for_that_reason() {
    let server = TestServer::start().await;
    let endpoint = fated(&server.endpoint, Fate::Silent).await;
    let mut config = config(&endpoint);
    config.transport.http2.keep_alive_interval = Some(Duration::from_millis(100));
    config.transport.http2.keep_alive_timeout = Duration::from_millis(200);
    config.transport.http2.keep_alive_while_idle = true;
    let channel = channel_with(config).expect("a channel");

    let (mut send, mut recv, _control) = channel
        .start_call(CallStartOptions::new(CHAT))
        .expect("the call starts")
        .split();
    send.send_message(Bytes::from_static(b"x"))
        .await
        .expect("sent");
    let _ = tokio::time::timeout(Duration::from_secs(10), recv.next_message()).await;

    until("the session closed", || {
        closed(&channel.stats(), CloseReason::KeepaliveTimeout) == 1
    })
    .await;
}

#[tokio::test]
async fn a_session_the_peer_closes_with_no_goaway_is_closed_for_that_reason() {
    let server = TestServer::start().await;
    let endpoint = fated(&server.endpoint, Fate::Closes).await;
    let channel = channel(&endpoint);

    let (mut send, mut recv, _control) = channel
        .start_call(CallStartOptions::new(CHAT))
        .expect("the call starts")
        .split();
    send.send_message(Bytes::from_static(b"x"))
        .await
        .expect("sent");
    let _ = tokio::time::timeout(Duration::from_secs(10), recv.next_message()).await;

    until("the session closed", || {
        closed(&channel.stats(), CloseReason::PeerClosed) == 1
    })
    .await;
}

#[tokio::test]
async fn a_session_that_fails_to_read_is_closed_as_an_io_error() {
    let server = TestServer::start().await;
    let endpoint = fated(&server.endpoint, Fate::Resets).await;
    let channel = channel(&endpoint);

    let (mut send, mut recv, _control) = channel
        .start_call(CallStartOptions::new(CHAT))
        .expect("the call starts")
        .split();
    send.send_message(Bytes::from_static(b"x"))
        .await
        .expect("sent");
    let _ = tokio::time::timeout(Duration::from_secs(10), recv.next_message()).await;

    until("the session closed", || {
        closed(&channel.stats(), CloseReason::IoError) == 1
    })
    .await;
}

/// A server that answers its SETTINGS and then breaks the protocol: DATA on stream zero.
async fn lawless() -> String {
    let (listener, endpoint) = loopback().await;
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut preface = [0u8; 24];
                if stream.read_exact(&mut preface).await.is_err() {
                    return;
                }
                let broken = [
                    &[0, 0, 0, 4, 0, 0, 0, 0, 0][..],
                    &[0, 0, 1, 0, 0, 0, 0, 0, 0, 0][..],
                ]
                .concat();
                let _ = stream.write_all(&broken).await;
                let mut drained = [0u8; 1024];
                while stream.read(&mut drained).await.is_ok_and(|read| read > 0) {}
            });
        }
    });
    endpoint
}

#[tokio::test]
async fn a_session_that_breaks_the_protocol_is_closed_for_that_reason() {
    let endpoint = lawless().await;
    let channel = channel(&endpoint);

    let _ = unary(&channel, CallStartOptions::new(ECHO), Bytes::new()).await;

    until("the session closed", || {
        closed(&channel.stats(), CloseReason::ProtocolError) == 1
    })
    .await;
}

/// Every connection opens and closes once, so open connections never drift.
#[tokio::test]
async fn open_connections_are_the_dials_that_succeeded_less_the_closes() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);
    echo(&channel, b"x").await;

    let stats = channel.stats();
    let open = stats.dials_succeeded - stats.connections_closed.iter().sum::<u64>();
    assert_eq!(open, 1);

    channel.close();
    until("the session closed", || {
        let stats = channel.stats();
        stats.dials_succeeded - stats.connections_closed.iter().sum::<u64>() == 0
    })
    .await;
}

fn judging(endpoint: &str) -> GrpcChannel {
    let mut config = config(endpoint);
    let mut retry = RetryConfig::default();
    retry.initial_backoff = Duration::from_millis(2);
    retry.max_backoff = Duration::from_millis(5);
    config.retry = Some(retry);
    let mut adaptive = AdaptiveConfig::default();
    adaptive.slack = 2;
    adaptive.window = Duration::from_secs(10);
    config.adaptive = Some(adaptive);
    channel_with(config).expect("a channel")
}

/// The gauges read the estimate when they are read: retries open, then closed by failures, and
/// the retries it refused counted.
#[tokio::test]
async fn the_gauges_read_the_state_of_the_estimate() {
    let server = TestServer::start().await;
    let channel = judging(&server.endpoint);

    let open = channel.stats();
    assert_eq!((open.channels_retries_closed, open.channels_capped), (0, 0));
    assert_eq!(open.throttle_cap_per_second, 0.0);

    for _ in 0..3 {
        unary(&channel, flaky("14", 1_000, &[]), Bytes::from_static(b"x")).await;
    }
    let closed = channel.stats();
    assert_eq!(closed.channels_retries_closed, 1);
    assert!(closed.retries_refused >= 1, "{}", closed.retries_refused);
    assert_eq!(closed.channels_capped, 0, "an outage caps nothing");

    // The same server answering fine reopens them.
    for _ in 0..8 {
        echo(&channel, b"x").await;
    }
    assert_eq!(channel.stats().channels_retries_closed, 0);
}

#[tokio::test]
async fn an_overloaded_server_caps_the_rate_and_the_gauge_says_so() {
    let server = TestServer::start().await;
    let channel = judging(&server.endpoint);

    for _ in 0..6 {
        unary(&channel, flaky("8", 1_000, &[]), Bytes::from_static(b"x")).await;
    }
    let stats = channel.stats();
    assert_eq!(stats.channels_capped, 1);
    assert!(stats.throttle_cap_per_second > 0.0);
}

#[tokio::test]
async fn channels_that_share_a_registry_are_read_as_one() {
    let server = TestServer::start().await;
    let metrics = Metrics::new();
    let mut channels = Vec::new();
    for _ in 0..2 {
        let mut config = config(&server.endpoint);
        config.metrics = Some(metrics.clone());
        channels.push(channel_with(config).expect("a channel"));
    }
    for channel in &channels {
        echo(channel, b"x").await;
    }

    assert_eq!(metrics.stats().calls_started, 2);
    assert_eq!(channels[0].stats(), metrics.stats());
    assert_eq!(channels[1].stats(), metrics.stats());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_multi_thread_runtime_adds_up() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);
    const TASKS: usize = 8;
    const CALLS: usize = 25;

    let tasks: Vec<_> = (0..TASKS)
        .map(|_| {
            let channel = channel.clone();
            tokio::spawn(async move {
                for _ in 0..CALLS {
                    echo(&channel, b"hello").await;
                }
            })
        })
        .collect();
    for task in tasks {
        task.await.expect("a task finished");
    }

    let stats = channel.stats();
    let total = (TASKS * CALLS) as u64;
    assert_eq!(stats.calls_started, total);
    assert_eq!(stats.ended_with(GrpcStatusCode::Ok), total);
    assert_eq!(
        (stats.messages_sent, stats.messages_received),
        (total, total)
    );
    assert_eq!(stats.message_bytes_raw, 5 * total);
    assert_eq!(stats.retries.iter().sum::<u64>(), 0);
    assert!(stats.dials_succeeded >= 1);
}
