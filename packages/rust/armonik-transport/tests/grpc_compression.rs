//! The compression of a call's messages: what a channel sends under `grpc-encoding`, what it
//! accepts under `grpc-accept-encoding`, and what each does to the limits and the replay.

mod common;

use std::time::Duration;

use armonik_transport::grpc::{
    CallStartOptions, Encoding, FramedMessage, GrpcChannel, GrpcChannelConfig, GrpcStatus,
    GrpcStatusCode, MetadataValue, ResponseHead, ResponseSink, RetryConfig,
};
use armonik_transport::http2::TransportConfig;
use bytes::Bytes;
use common::echo::*;
use http::Uri;

fn compressing(
    endpoint: &str,
    send: Option<Encoding>,
    accept: Option<Encoding>,
    change: impl FnOnce(&mut GrpcChannelConfig),
) -> GrpcChannel {
    let mut config = GrpcChannelConfig::new(TransportConfig::new(
        Uri::try_from(endpoint).expect("an endpoint"),
    ));
    config.send_encoding = send;
    config.accept_encoding = accept;
    change(&mut config);
    channel_with(config).expect("a channel")
}

fn sending(endpoint: &str) -> GrpcChannel {
    compressing(endpoint, Some(Encoding::Gzip), None, |_| {})
}

fn accepting(endpoint: &str) -> GrpcChannel {
    compressing(endpoint, None, Some(Encoding::Gzip), |_| {})
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

/// A call that sends one request, the way a unary call does.
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

async fn said_of(channel: &GrpcChannel, messages: &[&[u8]]) -> String {
    let (mut send, mut recv, _control) = channel
        .start_call(CallStartOptions::new(FRAMES))
        .expect("the call starts")
        .split();
    for message in messages {
        send.send_message(Bytes::copy_from_slice(message))
            .await
            .expect("sent");
    }
    let _ = send.end_send().await;
    let (_, heard, status) = read_to_terminal(&mut recv).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    String::from_utf8(heard.concat()).expect("text")
}

#[tokio::test]
async fn a_message_sent_compressed_is_inflated_by_the_server() {
    let server = TestServer::start().await;
    let channel = sending(&server.endpoint);

    for len in [0, 1, 100, 10_000, 200 * 1024, 3 * 1024 * 1024] {
        let message: Vec<u8> = (0..len).map(|at| b"abcde"[at % 5]).collect();

        let (_, messages, status) = unary(
            &channel,
            CallStartOptions::new(ECHO_GZIP),
            Bytes::from(message.clone()),
        )
        .await;
        assert_eq!(status.code, GrpcStatusCode::Ok, "{len}: {status}");
        assert_eq!(messages, vec![Bytes::from(message.clone())], "{len}");

        let (messages, status) =
            one_request(&channel, CallStartOptions::new(ECHO_GZIP), &message).await;
        assert_eq!(status.code, GrpcStatusCode::Ok, "{len}: {status}");
        assert_eq!(messages, vec![Bytes::from(message)], "one request, {len}");
    }
}

#[tokio::test]
async fn the_server_sees_the_encoding_and_the_compressed_flag() {
    let server = TestServer::start().await;
    let text = b"abc".repeat(1000);

    let said = said_of(&sending(&server.endpoint), &[&text]).await;
    assert!(said.starts_with("encoding=gzip frames=1:"), "{said}");

    let wire: usize = said["encoding=gzip frames=1:".len()..]
        .parse()
        .expect("a length");
    assert!(
        wire < 100,
        "3000 bytes of a pattern are a few dozen: {wire}"
    );

    let plain = said_of(&channel(&server.endpoint), &[&text]).await;
    assert_eq!(plain, "encoding=none frames=0:3000");
}

/// The flag is a message's own: a request has the encoding in its head however few of its
/// messages are compressed.
#[tokio::test]
async fn each_message_is_flagged_by_what_it_gains() {
    let server = TestServer::start().await;
    let channel = sending(&server.endpoint);
    let first = b"a".repeat(3000);
    let last = b"b".repeat(3000);

    let said = said_of(&channel, &[&first, b"hi", b"", &last]).await;

    let frames = said
        .strip_prefix("encoding=gzip frames=")
        .unwrap_or_else(|| panic!("{said}"));
    let flags: Vec<&str> = frames.split(',').collect();
    assert_eq!(flags.len(), 4, "{said}");
    assert!(flags[0].starts_with("1:"), "{said}");
    assert_eq!(flags[1], "0:2", "{said}");
    assert_eq!(flags[2], "0:0", "{said}");
    assert!(flags[3].starts_with("1:"), "{said}");
}

#[tokio::test]
async fn a_one_request_call_is_compressed_like_a_stream() {
    let server = TestServer::start().await;
    let channel = sending(&server.endpoint);

    let (messages, status) = one_request(
        &channel,
        CallStartOptions::new(FRAMES),
        &b"abc".repeat(1000),
    )
    .await;

    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    let said = String::from_utf8(messages.concat()).expect("text");
    assert!(said.starts_with("encoding=gzip frames=1:"), "{said}");
}

#[tokio::test]
async fn a_server_that_does_not_accept_the_encoding_ends_the_call_unimplemented() {
    let server = TestServer::start().await;
    let channel = sending(&server.endpoint);

    let (_, _, status) = unary(
        &channel,
        CallStartOptions::new(ECHO),
        Bytes::from(b"abc".repeat(1000)),
    )
    .await;

    assert_eq!(status.code, GrpcStatusCode::Unimplemented, "{status}");
    assert!(status.message.contains("gzip"), "{status}");
}

/// The limit is on a message as the caller wrote it, so what compresses to a few bytes is still
/// refused, and none of it is sent.
#[tokio::test]
async fn the_send_limit_is_on_the_message_before_it_is_compressed() {
    let server = TestServer::start().await;
    let channel = compressing(&server.endpoint, Some(Encoding::Gzip), None, |config| {
        config.max_send_message_size = Some(1000)
    });
    let text = b"a".repeat(3000);

    let (mut send, mut recv, _control) = channel
        .start_call(CallStartOptions::new(ECHO_GZIP))
        .expect("the call starts")
        .split();
    assert!(send.send_message(Bytes::from(text.clone())).await.is_err());
    let _ = send.end_send().await;
    let (_, _, status) = read_to_terminal(&mut recv).await;
    assert_eq!(status.code, GrpcStatusCode::ResourceExhausted, "{status}");

    let (messages, status) = one_request(&channel, CallStartOptions::new(ECHO_GZIP), &text).await;
    assert_eq!(status.code, GrpcStatusCode::ResourceExhausted, "{status}");
    assert!(messages.is_empty());

    let (messages, status) =
        one_request(&channel, CallStartOptions::new(ECHO_GZIP), &text[..900]).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(messages, vec![Bytes::from(text[..900].to_vec())]);
}

fn flaky(method: &str, key: &str) -> CallStartOptions {
    let mut options = CallStartOptions::new(method);
    for (name, value) in [("x-flaky-key", key), ("x-fail-times", "1")] {
        options
            .metadata
            .append(name, MetadataValue::Ascii(value.to_owned()))
            .expect("a header");
    }
    options
}

/// A call keeps what it sent for a replay up to a ceiling, and keeps it as it went out: 100 KiB of
/// a pattern is a few hundred bytes compressed, which a ceiling of 1 KiB holds.
#[tokio::test]
async fn a_replay_holds_the_message_compressed() {
    let server = TestServer::start().await;
    let text = b"abc".repeat(100 * 1024 / 3);
    let retry = |config: &mut GrpcChannelConfig| {
        let mut retry = RetryConfig::default();
        retry.initial_backoff = Duration::from_millis(10);
        retry.max_backoff = Duration::from_millis(50);
        retry.call_replay_bytes = 1024;
        config.retry = Some(retry);
    };

    let plain = compressing(&server.endpoint, None, None, retry);
    let (_, messages, status) = unary(
        &plain,
        flaky(FLAKY, "replay-plain"),
        Bytes::from(text.clone()),
    )
    .await;
    assert_eq!(status.code, GrpcStatusCode::Unavailable, "{status}");
    assert!(messages.is_empty());
    assert_eq!(flaky_seen("replay-plain").len(), 1, "too large to keep");

    let squeezed = compressing(&server.endpoint, Some(Encoding::Gzip), None, retry);
    let (_, messages, status) = unary(
        &squeezed,
        flaky(FLAKY, "replay-stream"),
        Bytes::from(text.clone()),
    )
    .await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(messages, vec![Bytes::from(text.clone())]);
    assert_eq!(flaky_seen("replay-stream").len(), 2, "kept, and sent again");

    let (messages, status) = one_request(&squeezed, flaky(FLAKY, "replay-one"), &text).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(messages, vec![Bytes::from(text)]);
    assert_eq!(flaky_seen("replay-one").len(), 2);
}

#[tokio::test]
async fn the_request_states_what_the_channel_sends_and_accepts() {
    let server = TestServer::start().await;
    let seen = |channel: GrpcChannel| async move {
        let (_, messages, status) = unary(
            &channel,
            CallStartOptions::new("/raw/EchoHeaders"),
            Bytes::from_static(b"x"),
        )
        .await;
        assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
        String::from_utf8(messages.concat().to_vec()).expect("the headers as text")
    };

    let default = seen(channel(&server.endpoint)).await;
    assert!(
        default.contains("grpc-accept-encoding=identity"),
        "{default}"
    );
    assert!(!default.contains("grpc-encoding"), "{default}");

    let both = seen(compressing(
        &server.endpoint,
        Some(Encoding::Gzip),
        Some(Encoding::Gzip),
        |_| {},
    ))
    .await;
    assert!(
        both.contains("grpc-accept-encoding=gzip,identity"),
        "{both}"
    );
    assert!(both.contains("grpc-encoding=gzip"), "{both}");

    let only_sending = seen(sending(&server.endpoint)).await;
    assert!(
        only_sending.contains("grpc-accept-encoding=identity"),
        "{only_sending}"
    );
    assert!(
        only_sending.contains("grpc-encoding=gzip"),
        "{only_sending}"
    );
}

/// A server that compresses its answers does so only in an encoding the channel advertised, so a
/// channel that advertised none is answered with messages it reads as they are.
#[tokio::test]
async fn a_channel_is_answered_in_the_encodings_it_accepts() {
    let server = TestServer::start().await;
    let text = Bytes::from(b"abc".repeat(1000));

    for channel in [
        sending(&server.endpoint),
        accepting(&server.endpoint),
        compressing(
            &server.endpoint,
            Some(Encoding::Gzip),
            Some(Encoding::Gzip),
            |_| {},
        ),
    ] {
        let (_, messages, status) =
            unary(&channel, CallStartOptions::new(ECHO_GZIP), text.clone()).await;
        assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
        assert_eq!(messages, vec![text.clone()]);
    }
}

#[tokio::test]
async fn a_compressed_answer_is_inflated_when_the_channel_accepts_its_encoding() {
    let server = TestServer::start().await;
    let channel = accepting(&server.endpoint);

    let (_, messages, status) = unary(
        &channel,
        CallStartOptions::new("/raw/GzipReply"),
        Bytes::from_static(b"x"),
    )
    .await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(messages, vec![Bytes::from(b"squeezed ".repeat(100))]);

    let (_, messages, status) = unary(
        &channel,
        CallStartOptions::new("/raw/GzipReplyLeftPlain"),
        Bytes::from_static(b"x"),
    )
    .await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(messages, vec![Bytes::from_static(b"plain")]);
}

/// The compression document faults a message compressed in an encoding the client does not
/// support, `INTERNAL`. One this channel was not set to accept is such a message, whatever the
/// server is able to send.
#[tokio::test]
async fn a_message_in_an_encoding_the_channel_does_not_accept_ends_the_call_internal() {
    let server = TestServer::start().await;

    for (channel, method) in [
        (channel(&server.endpoint), "/raw/GzipReply"),
        (sending(&server.endpoint), "/raw/GzipReply"),
        (accepting(&server.endpoint), "/raw/BrotliReply"),
    ] {
        let (_, messages, status) = unary(
            &channel,
            CallStartOptions::new(method),
            Bytes::from_static(b"x"),
        )
        .await;

        assert_eq!(status.code, GrpcStatusCode::Internal, "{method}: {status}");
        assert!(status.message.contains("compress"), "{status}");
        assert!(messages.is_empty());
    }
}

/// The head that names an encoding is not what the document faults: a call under it that
/// compresses nothing is the call it would have been, and so is an error status.
#[tokio::test]
async fn a_head_naming_an_unaccepted_encoding_over_nothing_compressed_is_left_alone() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    let (_, messages, status) = unary(
        &channel,
        CallStartOptions::new("/raw/BrotliReplyLeftPlain"),
        Bytes::from_static(b"x"),
    )
    .await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(messages, vec![Bytes::from_static(b"plain")]);

    let (_, messages, status) = unary(
        &channel,
        CallStartOptions::new("/raw/GzipTrailersOnlyUnavailable"),
        Bytes::from_static(b"x"),
    )
    .await;
    assert_eq!(status.code, GrpcStatusCode::Unavailable, "{status}");
    assert!(messages.is_empty());
}

#[tokio::test]
async fn a_message_that_does_not_inflate_ends_the_call_internal() {
    let server = TestServer::start().await;

    let (_, messages, status) = unary(
        &accepting(&server.endpoint),
        CallStartOptions::new("/raw/GzipReplyThatDoesNotInflate"),
        Bytes::from_static(b"x"),
    )
    .await;

    assert_eq!(status.code, GrpcStatusCode::Internal, "{status}");
    assert!(status.message.contains("decompress"), "{status}");
    assert!(messages.is_empty());
}

/// The limit is on the message as the application reads it. Eight MiB of zeros is a few KiB on
/// the wire, which no limit on the wire alone would stop.
#[tokio::test]
async fn the_receive_limit_is_on_the_inflated_message() {
    let server = TestServer::start().await;
    let eight_mib = 8 * 1024 * 1024;

    let (_, messages, status) = unary(
        &accepting(&server.endpoint),
        CallStartOptions::new("/raw/GzipReplyOfEightMiB"),
        Bytes::from_static(b"x"),
    )
    .await;
    assert_eq!(status.code, GrpcStatusCode::ResourceExhausted, "{status}");
    assert!(status.message.contains("4194304"), "{status}");
    assert!(messages.is_empty());

    let raised = compressing(&server.endpoint, None, Some(Encoding::Gzip), |config| {
        config.max_recv_message_size = eight_mib
    });
    let (_, messages, status) = unary(
        &raised,
        CallStartOptions::new("/raw/GzipReplyOfEightMiB"),
        Bytes::from_static(b"x"),
    )
    .await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].len(), eight_mib);
}
