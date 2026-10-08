//! When a call's messages are compressed: after its first attempt has taken its turn at the rate
//! limit, so that a call that waits has compressed nothing, and the encoding is the one the
//! channel knows by then.
//!
//! The count of compressions is the process's, so the tests hold one lock and run one at a time.

mod common;

use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use armonik_transport::grpc::{
    CallStartOptions, Deadline, Encoding, FramedMessage, GrpcChannel, GrpcChannelConfig,
    GrpcStatus, GrpcStatusCode, RateLimitConfig, ResponseHead, ResponseSink,
};
use armonik_transport::hooks;
use armonik_transport::http2::TransportConfig;
use bytes::Bytes;
use common::echo::*;
use http::Uri;

static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn one_at_a_time() -> MutexGuard<'static, ()> {
    ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A channel that sends gzip and starts one request in each window of `window`.
fn limited(endpoint: &str, window: Duration) -> GrpcChannel {
    let mut config = GrpcChannelConfig::new(TransportConfig::new(
        Uri::try_from(endpoint).expect("an endpoint"),
    ));
    config.send_encoding = Some(Encoding::Gzip);
    config.rate_limit = Some(RateLimitConfig::new(1, window));
    channel_with(config).expect("a channel")
}

fn with_deadline(method: &str, after: Duration) -> CallStartOptions {
    let mut options = CallStartOptions::new(method);
    options.deadline = Some(Deadline::Timeout(after));
    options
}

/// What a call that sends one request heard: its messages, and its status.
struct Heard {
    messages: Vec<Bytes>,
    done: Option<tokio::sync::oneshot::Sender<(Vec<Bytes>, GrpcStatus)>>,
}

impl ResponseSink for Heard {
    async fn head(&mut self, _: ResponseHead) -> Result<(), GrpcStatus> {
        Ok(())
    }

    async fn message(&mut self, data: Bytes) -> Result<(), GrpcStatus> {
        self.messages.push(data);
        Ok(())
    }

    fn flush(&mut self) {}

    async fn end(mut self, status: GrpcStatus, _: Option<ResponseHead>) {
        if let Some(done) = self.done.take() {
            let _ = done.send((std::mem::take(&mut self.messages), status));
        }
    }
}

/// A call that sends one request of `message`, the way a unary call does.
async fn one_request(
    channel: &GrpcChannel,
    options: CallStartOptions,
    message: &[u8],
) -> (Vec<Bytes>, GrpcStatus) {
    let (request, _control, driver) = channel
        .prepare_one_request_call(options)
        .expect("an open channel");
    assert!(request.give(|| FramedMessage::copy_of(message).expect("a message")));
    let (done, heard) = tokio::sync::oneshot::channel();
    driver
        .drive(Heard {
            messages: Vec::new(),
            done: Some(done),
        })
        .await;
    heard.await.expect("the sink heard the end")
}

/// A call that streams `message` and ends its send, the way `start_call` serves a stream.
async fn streamed(
    channel: &GrpcChannel,
    options: CallStartOptions,
    message: &[u8],
) -> (Vec<Bytes>, GrpcStatus) {
    let (mut send, mut recv, _control) = channel
        .start_call(options)
        .expect("the call starts")
        .split();
    let _ = send.send_message(Bytes::copy_from_slice(message)).await;
    let _ = send.end_send().await;
    let (_, heard, status) = read_to_terminal(&mut recv).await;
    (heard, status)
}

fn text(heard: &[Bytes]) -> String {
    String::from_utf8(heard.concat()).expect("text")
}

/// A message that compresses.
fn squeezable() -> Vec<u8> {
    b"abc".repeat(1000)
}

/// A call that has to wait for its turn behind another has compressed nothing when its deadline
/// ends it; the one that went first has compressed its message, once.
#[tokio::test]
async fn a_call_that_waits_for_its_turn_compresses_nothing_when_its_deadline_ends_it() {
    let _alone = one_at_a_time();
    let server = TestServer::start().await;
    let channel = limited(&server.endpoint, Duration::from_secs(30));
    let message = squeezable();
    let before = hooks::compressions();

    let (first, queued) = tokio::join!(
        one_request(&channel, CallStartOptions::new(ECHO_COMPRESSED), &message),
        one_request(
            &channel,
            with_deadline(ECHO_COMPRESSED, Duration::from_millis(200)),
            &message
        ),
    );
    assert_eq!(first.1.code, GrpcStatusCode::Ok, "{}", first.1);
    assert_eq!(
        queued.1.code,
        GrpcStatusCode::DeadlineExceeded,
        "{}",
        queued.1
    );
    assert_eq!(hooks::compressions() - before, 1, "the first call's alone");

    // The same for a call that streams its messages.
    let before = hooks::compressions();
    let queued = streamed(
        &channel,
        with_deadline(FRAMES, Duration::from_millis(200)),
        &message,
    )
    .await;
    assert_eq!(
        queued.1.code,
        GrpcStatusCode::DeadlineExceeded,
        "{}",
        queued.1
    );
    assert_eq!(
        hooks::compressions() - before,
        0,
        "nothing for a call that waited"
    );
}

/// A call cancelled while it waits has compressed nothing either.
#[tokio::test]
async fn a_call_cancelled_while_it_waits_for_its_turn_compresses_nothing() {
    let _alone = one_at_a_time();
    let server = TestServer::start().await;
    let channel = limited(&server.endpoint, Duration::from_secs(30));
    let message = squeezable();

    // The one turn of the window.
    let (first, _) = streamed(&channel, CallStartOptions::new(FRAMES), b"x").await;
    assert!(!first.is_empty());
    let before = hooks::compressions();

    let (mut send, mut recv, control) = channel
        .start_call(CallStartOptions::new(FRAMES))
        .expect("the call starts")
        .split();
    let _ = send.send_message(Bytes::copy_from_slice(&message)).await;
    let _ = send.end_send().await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    control.cancel();
    let (_, _, status) = read_to_terminal(&mut recv).await;

    assert_eq!(status.code, GrpcStatusCode::Cancelled, "{status}");
    assert_eq!(hooks::compressions() - before, 0);
}

/// A channel that learns, while a call waits, that the server does not take the encoding sends the
/// call as it is: the call's turn is when the encoding is chosen.
#[tokio::test]
async fn a_call_queued_while_the_channel_learns_the_server_lacks_the_encoding_goes_out_as_identity()
{
    let _alone = one_at_a_time();
    let server = TestServer::start().await;
    let message = squeezable();

    // A one-request call.
    let channel = limited(&server.endpoint, Duration::from_millis(1500));
    let (taught, queued) = tokio::join!(
        one_request(
            &channel,
            CallStartOptions::new("/raw/Accepts:identity"),
            b"x"
        ),
        one_request(
            &channel,
            CallStartOptions::new("/raw/EchoHeaders"),
            &message
        ),
    );
    assert_eq!(taught.1.code, GrpcStatusCode::Ok, "{}", taught.1);
    assert_eq!(queued.1.code, GrpcStatusCode::Ok, "{}", queued.1);
    let seen = text(&queued.0);
    assert!(!seen.contains("grpc-encoding"), "{seen}");

    // A call that streams.
    let channel = limited(&server.endpoint, Duration::from_millis(1500));
    let (taught, queued) = tokio::join!(
        one_request(
            &channel,
            CallStartOptions::new("/raw/Accepts:identity"),
            b"x"
        ),
        streamed(&channel, CallStartOptions::new(FRAMES), &message),
    );
    assert_eq!(taught.1.code, GrpcStatusCode::Ok, "{}", taught.1);
    assert_eq!(queued.1.code, GrpcStatusCode::Ok, "{}", queued.1);
    assert!(
        text(&queued.0).starts_with("encoding=none"),
        "{}",
        text(&queued.0)
    );

    // And one that is queued while the channel knows nothing against it goes out compressed.
    let channel = limited(&server.endpoint, Duration::from_millis(1500));
    let (_, queued) = tokio::join!(
        one_request(&channel, CallStartOptions::new("/raw/EchoHeaders"), b"x"),
        streamed(&channel, CallStartOptions::new(FRAMES), &message),
    );
    assert!(
        text(&queued.0).starts_with("encoding=gzip"),
        "{}",
        text(&queued.0)
    );
}
