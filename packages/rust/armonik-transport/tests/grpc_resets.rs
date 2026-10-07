//! How many RST_STREAM frames the engine sends: none for a call that ends as gRPC means it to, of
//! any cardinality, nor for one whose response ends while its request is still open, which it
//! half-closes; one for each call it stops - a cancel, a deadline, a failure of its own. A server
//! counting resets per connection, as rapid-reset defences do, sees only those.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use armonik_transport::grpc::{
    CallControl, CallStartOptions, Deadline, GrpcChannel, GrpcChannelConfig, GrpcStatusCode,
};
use armonik_transport::http2::TransportConfig;
use bytes::Bytes;
use common::echo::*;
use http::Uri;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const DATA: u8 = 0x0;
const HEADERS: u8 = 0x1;
const RST_STREAM: u8 = 0x3;
const END_STREAM: u8 = 0x1;
const CANCEL: u32 = 0x8;
const PREFACE: usize = 24;

/// What a client sent, frame by frame.
#[derive(Default)]
struct Sent {
    heads: AtomicUsize,
    half_closes: AtomicUsize,
    resets: AtomicUsize,
    cancels: AtomicUsize,
}

/// A proxy in front of `upstream` that counts what its clients send: the HEADERS that open
/// streams, the frames that half-close them, and the RST_STREAM frames.
struct Census {
    endpoint: String,
    sent: Arc<Sent>,
}

impl Census {
    async fn start(upstream: &str) -> Self {
        let upstream = upstream
            .strip_prefix("http://")
            .expect("a plain endpoint")
            .to_owned();
        let (listener, endpoint) = loopback().await;
        let sent = Arc::new(Sent::default());
        let counted = sent.clone();
        tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                let server = TcpStream::connect(&upstream)
                    .await
                    .expect("the upstream accepts");
                // Neither side holds a frame back waiting for an acknowledgement.
                client.set_nodelay(true).expect("no delay");
                server.set_nodelay(true).expect("no delay");
                let counted = counted.clone();
                tokio::spawn(async move {
                    let (mut from_client, mut to_client) = client.into_split();
                    let (mut from_server, mut to_server) = server.into_split();
                    tokio::spawn(async move {
                        let _ = tokio::io::copy(&mut from_server, &mut to_client).await;
                    });
                    let mut preface = [0; PREFACE];
                    if from_client.read_exact(&mut preface).await.is_err()
                        || to_server.write_all(&preface).await.is_err()
                    {
                        return;
                    }
                    loop {
                        let mut frame = vec![0; 9];
                        if from_client.read_exact(&mut frame).await.is_err() {
                            return;
                        }
                        let length = u32::from_be_bytes([0, frame[0], frame[1], frame[2]]) as usize;
                        let (kind, flags) = (frame[3], frame[4]);
                        frame.resize(9 + length, 0);
                        if from_client.read_exact(&mut frame[9..]).await.is_err() {
                            return;
                        }
                        match kind {
                            HEADERS => {
                                counted.heads.fetch_add(1, Ordering::SeqCst);
                            }
                            RST_STREAM => {
                                counted.resets.fetch_add(1, Ordering::SeqCst);
                                if frame[9..13] == CANCEL.to_be_bytes() {
                                    counted.cancels.fetch_add(1, Ordering::SeqCst);
                                }
                            }
                            _ => {}
                        }
                        if (kind == DATA || kind == HEADERS) && flags & END_STREAM != 0 {
                            counted.half_closes.fetch_add(1, Ordering::SeqCst);
                        }
                        if to_server.write_all(&frame).await.is_err() {
                            return;
                        }
                    }
                });
            }
        });
        Self { endpoint, sent }
    }

    fn heads(&self) -> usize {
        self.sent.heads.load(Ordering::SeqCst)
    }

    fn half_closes(&self) -> usize {
        self.sent.half_closes.load(Ordering::SeqCst)
    }

    fn resets(&self) -> usize {
        self.sent.resets.load(Ordering::SeqCst)
    }

    fn cancels(&self) -> usize {
        self.sent.cancels.load(Ordering::SeqCst)
    }

    /// Until `heads` streams have been opened.
    async fn opened(&self, heads: usize) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.heads() < heads {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("the call's head reaches the server");
    }
}

/// A call to `method` sending `messages`, half-closed, read to its end.
async fn call(channel: &GrpcChannel, method: &str, messages: &[&'static str]) -> GrpcStatusCode {
    let (mut send, mut recv, _control) = channel
        .start_call(CallStartOptions::new(method))
        .expect("the call starts")
        .split();
    for message in messages {
        send.send_message(Bytes::from_static(message.as_bytes()))
            .await
            .expect("the message is accepted");
    }
    send.end_send().await.expect("the half-close");
    read_to_terminal(&mut recv).await.2.code
}

/// Whether the call's request ends as a reset, which the call settles before its status is out.
///
/// The frames say as much, but only as a race: a request that ends whole is a half-close that
/// reaches the wire first only when the connection writes in the instants between the request's
/// end and the stream's release, which h2 turns into a reset.
#[cfg(feature = "test-hooks")]
fn assert_cut(control: &CallControl, cut: bool, call: &str) {
    assert_eq!(control.is_cut(), cut, "{call}");
}

#[cfg(not(feature = "test-hooks"))]
fn assert_cut(_: &CallControl, _: bool, _: &str) {}

/// What a frame could still be on its way with: the engine writes it on its own task.
async fn settled() {
    tokio::time::sleep(Duration::from_millis(100)).await;
}

#[tokio::test]
async fn calls_that_end_normally_send_no_reset() {
    let server = TestServer::start().await;
    let census = Census::start(&server.endpoint).await;
    let channel = channel(&census.endpoint);

    for _ in 0..50 {
        assert_eq!(call(&channel, ECHO, &["x"]).await, GrpcStatusCode::Ok);
        assert_eq!(call(&channel, FAN, &["a,b,c"]).await, GrpcStatusCode::Ok);
        assert_eq!(
            call(&channel, COLLECT, &["a", "b", "c"]).await,
            GrpcStatusCode::Ok
        );
        assert_eq!(
            call(&channel, CHAT, &["a", "b", "c"]).await,
            GrpcStatusCode::Ok
        );
    }
    // A status other than OK ends the call too, and is no reason to reset it.
    for _ in 0..50 {
        assert_eq!(
            call(&channel, FAIL, &["x"]).await,
            GrpcStatusCode::PermissionDenied
        );
    }
    settled().await;
    assert_eq!(census.heads(), 250);
    assert_eq!(census.resets(), 0);
}

#[tokio::test]
async fn a_cancelled_call_sends_one_reset() {
    let server = TestServer::start().await;
    let census = Census::start(&server.endpoint).await;
    let channel = channel(&census.endpoint);

    for opened in 1..=10 {
        let (mut send, mut recv, control) = channel
            .start_call(CallStartOptions::new(SLOW))
            .expect("the call starts")
            .split();
        send.send_message(Bytes::from_static(b"x"))
            .await
            .expect("the message is accepted");
        send.end_send().await.expect("the half-close");
        // On the wire first: a call cancelled before its head goes out sends nothing at all.
        census.opened(opened).await;
        control.cancel();
        ends_cancelled(&mut recv, "the cancelled call ends").await;
    }
    settled().await;
    assert_eq!(census.resets(), 10);
}

/// A call cancelled while its request is open is reset, never half-closed: a server that took an
/// END_STREAM for the end of the request would take the messages before it for all of them.
#[tokio::test]
async fn a_call_cancelled_mid_request_is_reset_not_half_closed() {
    let server = TestServer::start().await;
    let census = Census::start(&server.endpoint).await;
    let channel = channel(&census.endpoint);

    for opened in 1..=10 {
        let (mut send, mut recv, control) = channel
            .start_call(CallStartOptions::new(COLLECT))
            .expect("the call starts")
            .split();
        send.send_message(Bytes::from_static(b"x"))
            .await
            .expect("the message is accepted");
        census.opened(opened).await;
        control.cancel();
        ends_cancelled(&mut recv, "the cancelled call ends").await;
        drop(send);
    }
    settled().await;
    assert_eq!(census.half_closes(), 0);
    assert_eq!(census.resets(), 10);
    assert_eq!(census.cancels(), 10);
}

/// The same once the response is under way: the peer has answered, and the request is still open.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_call_cancelled_mid_request_after_an_answer_is_reset_not_half_closed() {
    let server = TestServer::start().await;
    let census = Census::start(&server.endpoint).await;
    let channel = channel(&census.endpoint);

    for _ in 0..50 {
        let (mut send, mut recv, control) = channel
            .start_call(CallStartOptions::new(CHAT))
            .expect("the call starts")
            .split();
        send.send_message(Bytes::from_static(b"x"))
            .await
            .expect("the message is accepted");
        recv.recv_head().await.expect("the head");
        match recv.next_message().await {
            Ok(armonik_transport::grpc::RecvResult::Message(_)) => {}
            other => panic!("an echo: {other:?}"),
        }
        control.cancel();
        ends_cancelled(&mut recv, "the cancelled call ends").await;
        drop(send);
    }
    settled().await;
    assert_eq!(census.half_closes(), 0);
    assert_eq!(census.resets(), 50);
    assert_eq!(census.cancels(), 50);
}

#[tokio::test]
async fn a_call_past_its_deadline_sends_one_reset() {
    let server = TestServer::start().await;
    let census = Census::start(&server.endpoint).await;
    let channel = channel(&census.endpoint);

    for opened in 1..=10 {
        let mut options = CallStartOptions::new(SLOW);
        // Long enough for the head to go out first, which the count below checks.
        options.deadline = Some(Deadline::Timeout(Duration::from_millis(300)));
        let (_, _, status) = unary(&channel, options, Bytes::from_static(b"x")).await;
        assert_eq!(status.code, GrpcStatusCode::DeadlineExceeded, "{status}");
        assert_eq!(
            census.heads(),
            opened,
            "the call expired before its head went out"
        );
    }
    settled().await;
    assert_eq!(census.resets(), 10);
}

/// A call the peer ended keeps its request's half-close when it is cancelled after, as a host
/// releasing a finished call does.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_call_cancelled_after_its_end_keeps_its_half_close() {
    let server = TestServer::start().await;
    let census = Census::start(&server.endpoint).await;
    let channel = channel(&census.endpoint);

    for _ in 0..50 {
        let (mut send, mut recv, control) = channel
            .start_call(CallStartOptions::new(ANSWER_EARLY))
            .expect("the call starts")
            .split();
        send.send_message(Bytes::from_static(b"x"))
            .await
            .expect("the message is accepted");
        let (_, _, terminal) = read_to_terminal(&mut recv).await;
        assert_eq!(terminal.code, GrpcStatusCode::Ok, "{terminal}");
        control.cancel();
        drop(recv);
        drop(send);
    }
    settled().await;
    assert_eq!(census.half_closes(), 50);
    assert_eq!(census.resets(), 0);
}

/// A deadline stops a call as a cancel does: a request still open is reset, never half-closed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_call_past_its_deadline_mid_request_is_reset_not_half_closed() {
    let server = TestServer::start().await;
    let census = Census::start(&server.endpoint).await;
    let channel = channel(&census.endpoint);

    for _ in 0..20 {
        let mut options = CallStartOptions::new(CHAT);
        options.deadline = Some(Deadline::Timeout(Duration::from_millis(200)));
        let (mut send, mut recv, _control) = channel
            .start_call(options)
            .expect("the call starts")
            .split();
        send.send_message(Bytes::from_static(b"x"))
            .await
            .expect("the message is accepted");
        let (_, _, status) = read_to_terminal(&mut recv).await;
        assert_eq!(status.code, GrpcStatusCode::DeadlineExceeded, "{status}");
        drop(send);
    }
    settled().await;
    assert_eq!(census.half_closes(), 0);
    assert_eq!(census.resets(), 20);
    assert_eq!(census.cancels(), 20);
}

/// A call whose response is whole while its request is still open has its request half-closed
/// and not reset.
#[tokio::test]
async fn a_call_answered_before_its_request_ends_half_closes_it() {
    let server = TestServer::start().await;
    let census = Census::start(&server.endpoint).await;
    let channel = channel(&census.endpoint);

    for _ in 0..10 {
        let (mut send, mut recv, control) = channel
            .start_call(CallStartOptions::new(ANSWER_EARLY))
            .expect("the call starts")
            .split();
        send.send_message(Bytes::from_static(b"x"))
            .await
            .expect("the message is accepted");
        let (_, _, terminal) = read_to_terminal(&mut recv).await;
        assert_eq!(terminal.code, GrpcStatusCode::Ok, "{terminal}");
        assert_cut(&control, false, ANSWER_EARLY);
    }
    settled().await;
    assert_eq!(census.half_closes(), 10);
    assert_eq!(census.resets(), 0);
}

/// A channel that takes no message larger than `max` bytes.
fn receiving_at_most(endpoint: &str, max: usize) -> GrpcChannel {
    let uri = Uri::try_from(endpoint).expect("the test server's endpoint");
    let mut config = GrpcChannelConfig::new(TransportConfig::new(uri));
    config.max_recv_message_size = max;
    channel_with(config).expect("a plain endpoint")
}

/// A call that ends this side on a response it refuses, while the peer has not ended it, is reset
/// like one stopped by a cancel: the request it leaves open must not end as a whole one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_message_past_the_limit_resets_the_request_it_leaves_open() {
    let server = TestServer::start().await;
    let census = Census::start(&server.endpoint).await;
    let channel = receiving_at_most(&census.endpoint, 1024);

    for _ in 0..20 {
        let (mut send, mut recv, control) = channel
            .start_call(CallStartOptions::new(CHAT))
            .expect("the call starts")
            .split();
        send.send_message(Bytes::from(vec![b'x'; 4096]))
            .await
            .expect("the message is accepted");
        let (_, messages, status) = read_to_terminal(&mut recv).await;
        assert_eq!(status.code, GrpcStatusCode::ResourceExhausted, "{status}");
        assert!(messages.is_empty(), "{messages:?}");
        assert_cut(&control, true, "a message past the limit");
        drop(send);
    }
    settled().await;
    assert_eq!(census.half_closes(), 0);
    assert_eq!(census.resets(), 20);
    assert_eq!(census.cancels(), 20);
}

/// The same for the second message of a call that answers once, which the engine refuses before
/// the decoder sees it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_message_resets_the_request_it_leaves_open() {
    let server = TestServer::start().await;
    let census = Census::start(&server.endpoint).await;
    let channel = channel(&census.endpoint);

    for _ in 0..20 {
        let mut options = CallStartOptions::new(CHAT);
        options.one_response = true;
        let (mut send, mut recv, control) = channel
            .start_call(options)
            .expect("the call starts")
            .split();
        for message in ["a", "b"] {
            send.send_message(Bytes::from_static(message.as_bytes()))
                .await
                .expect("the message is accepted");
        }
        let (_, messages, status) = read_to_terminal(&mut recv).await;
        assert_eq!(status.code, GrpcStatusCode::Internal, "{status}");
        assert_eq!(messages, vec![Bytes::from_static(b"a")]);
        assert_cut(&control, true, "a second message");
        drop(send);
    }
    settled().await;
    assert_eq!(census.half_closes(), 0);
    assert_eq!(census.resets(), 20);
    assert_eq!(census.cancels(), 20);
}

/// A status other than OK that the peer gave, in a Trailers-Only response or in the trailers after
/// a message, ends the call as the peer's own end does, and its request is half-closed: tonic
/// returns such a status as an error from the read, as it does a message this side refuses.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_call_the_peer_failed_while_its_request_is_open_half_closes_it() {
    let server = TestServer::start().await;
    let census = Census::start(&server.endpoint).await;
    let channel = channel(&census.endpoint);

    for (method, messages) in [(REFUSE_EARLY, 0), (FAIL_EARLY, 1)] {
        for _ in 0..10 {
            let (mut send, mut recv, control) = channel
                .start_call(CallStartOptions::new(method))
                .expect("the call starts")
                .split();
            send.send_message(Bytes::from_static(b"x"))
                .await
                .expect("the message is accepted");
            let (_, answered, status) = read_to_terminal(&mut recv).await;
            assert_eq!(status.code, GrpcStatusCode::Unavailable, "{status}");
            assert_eq!(answered.len(), messages, "{method}");
            assert_cut(&control, false, method);
        }
    }
    settled().await;
    assert_eq!(census.half_closes(), 20);
    assert_eq!(census.resets(), 0);
}
