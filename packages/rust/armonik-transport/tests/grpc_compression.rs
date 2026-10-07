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

const ALL: [Encoding; 3] = [Encoding::Gzip, Encoding::Deflate, Encoding::Zstd];

/// The name the wire gives an encoding.
fn named(encoding: Encoding) -> &'static str {
    match encoding {
        Encoding::Gzip => "gzip",
        Encoding::Deflate => "deflate",
        Encoding::Zstd => "zstd",
        _ => unreachable!("an encoding this test does not know"),
    }
}

fn compressing(
    endpoint: &str,
    send: Option<Encoding>,
    accept: &[Encoding],
    change: impl FnOnce(&mut GrpcChannelConfig),
) -> GrpcChannel {
    let mut config = GrpcChannelConfig::new(TransportConfig::new(
        Uri::try_from(endpoint).expect("an endpoint"),
    ));
    config.send_encoding = send;
    config.accept_encodings = accept.to_vec();
    change(&mut config);
    channel_with(config).expect("a channel")
}

fn sending(endpoint: &str, encoding: Encoding) -> GrpcChannel {
    compressing(endpoint, Some(encoding), &[], |_| {})
}

fn accepting(endpoint: &str, accepted: &[Encoding]) -> GrpcChannel {
    compressing(endpoint, None, accepted, |_| {})
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

    for encoding in ALL {
        let channel = sending(&server.endpoint, encoding);
        for len in [0, 1, 100, 10_000, 200 * 1024, 3 * 1024 * 1024] {
            let message: Vec<u8> = (0..len).map(|at| b"abcde"[at % 5]).collect();

            let (_, messages, status) = unary(
                &channel,
                CallStartOptions::new(ECHO_COMPRESSED),
                Bytes::from(message.clone()),
            )
            .await;
            assert_eq!(
                status.code,
                GrpcStatusCode::Ok,
                "{encoding:?} {len}: {status}"
            );
            assert_eq!(
                messages,
                vec![Bytes::from(message.clone())],
                "{encoding:?} {len}"
            );

            let (messages, status) =
                one_request(&channel, CallStartOptions::new(ECHO_COMPRESSED), &message).await;
            assert_eq!(
                status.code,
                GrpcStatusCode::Ok,
                "{encoding:?} {len}: {status}"
            );
            assert_eq!(
                messages,
                vec![Bytes::from(message)],
                "one request, {encoding:?} {len}"
            );
        }
    }
}

#[tokio::test]
async fn the_server_sees_the_encoding_and_the_compressed_flag() {
    let server = TestServer::start().await;
    let text = b"abc".repeat(1000);

    for encoding in ALL {
        let said = said_of(&sending(&server.endpoint, encoding), &[&text]).await;
        let prefix = format!("encoding={} frames=1:", named(encoding));
        assert!(said.starts_with(&prefix), "{said}");

        let wire: usize = said[prefix.len()..].parse().expect("a length");
        assert!(
            wire < 100,
            "3000 bytes of a pattern are a few dozen: {encoding:?} {wire}"
        );
    }

    let plain = said_of(&channel(&server.endpoint), &[&text]).await;
    assert_eq!(plain, "encoding=none frames=0:3000");
}

/// The flag is a message's own: a request has the encoding in its head however few of its
/// messages are compressed.
#[tokio::test]
async fn each_message_is_flagged_by_what_it_gains() {
    let server = TestServer::start().await;
    let first = b"a".repeat(3000);
    let last = b"b".repeat(3000);

    for encoding in ALL {
        let channel = sending(&server.endpoint, encoding);

        let said = said_of(&channel, &[&first, b"hi", b"", &last]).await;

        let frames = said
            .strip_prefix(&format!("encoding={} frames=", named(encoding)))
            .unwrap_or_else(|| panic!("{said}"));
        let flags: Vec<&str> = frames.split(',').collect();
        assert_eq!(flags.len(), 4, "{said}");
        assert!(flags[0].starts_with("1:"), "{said}");
        assert_eq!(flags[1], "0:2", "{said}");
        assert_eq!(flags[2], "0:0", "{said}");
        assert!(flags[3].starts_with("1:"), "{said}");
    }
}

#[tokio::test]
async fn a_one_request_call_is_compressed_like_a_stream() {
    let server = TestServer::start().await;

    for encoding in ALL {
        let channel = sending(&server.endpoint, encoding);

        let (messages, status) = one_request(
            &channel,
            CallStartOptions::new(FRAMES),
            &b"abc".repeat(1000),
        )
        .await;

        assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
        let said = String::from_utf8(messages.concat()).expect("text");
        assert!(
            said.starts_with(&format!("encoding={} frames=1:", named(encoding))),
            "{said}"
        );
    }
}

#[tokio::test]
async fn a_server_that_does_not_accept_the_encoding_ends_the_call_unimplemented() {
    let server = TestServer::start().await;

    for encoding in ALL {
        let channel = sending(&server.endpoint, encoding);

        let (_, _, status) = unary(
            &channel,
            CallStartOptions::new(ECHO),
            Bytes::from(b"abc".repeat(1000)),
        )
        .await;

        assert_eq!(status.code, GrpcStatusCode::Unimplemented, "{status}");
        assert!(status.message.contains(named(encoding)), "{status}");
    }
}

/// The limit is on a message as the caller wrote it, so what compresses to a few bytes is still
/// refused, and none of it is sent.
#[tokio::test]
async fn the_send_limit_is_on_the_message_before_it_is_compressed() {
    let server = TestServer::start().await;
    let channel = compressing(&server.endpoint, Some(Encoding::Gzip), &[], |config| {
        config.max_send_message_size = Some(1000)
    });
    let text = b"a".repeat(3000);

    let (mut send, mut recv, _control) = channel
        .start_call(CallStartOptions::new(ECHO_COMPRESSED))
        .expect("the call starts")
        .split();
    assert!(send.send_message(Bytes::from(text.clone())).await.is_err());
    let _ = send.end_send().await;
    let (_, _, status) = read_to_terminal(&mut recv).await;
    assert_eq!(status.code, GrpcStatusCode::ResourceExhausted, "{status}");

    let (messages, status) =
        one_request(&channel, CallStartOptions::new(ECHO_COMPRESSED), &text).await;
    assert_eq!(status.code, GrpcStatusCode::ResourceExhausted, "{status}");
    assert!(messages.is_empty());

    let (messages, status) = one_request(
        &channel,
        CallStartOptions::new(ECHO_COMPRESSED),
        &text[..900],
    )
    .await;
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

    let plain = compressing(&server.endpoint, None, &[], retry);
    let (_, messages, status) = unary(
        &plain,
        flaky(FLAKY, "replay-plain"),
        Bytes::from(text.clone()),
    )
    .await;
    assert_eq!(status.code, GrpcStatusCode::Unavailable, "{status}");
    assert!(messages.is_empty());
    assert_eq!(flaky_seen("replay-plain").len(), 1, "too large to keep");

    for encoding in ALL {
        let squeezed = compressing(&server.endpoint, Some(encoding), &[], retry);
        let stream_key = format!("replay-stream-{encoding:?}");
        let (_, messages, status) = unary(
            &squeezed,
            flaky(FLAKY, &stream_key),
            Bytes::from(text.clone()),
        )
        .await;
        assert_eq!(status.code, GrpcStatusCode::Ok, "{encoding:?}: {status}");
        assert_eq!(messages, vec![Bytes::from(text.clone())]);
        assert_eq!(flaky_seen(&stream_key).len(), 2, "kept, and sent again");

        let one_key = format!("replay-one-{encoding:?}");
        let (messages, status) = one_request(&squeezed, flaky(FLAKY, &one_key), &text).await;
        assert_eq!(status.code, GrpcStatusCode::Ok, "{encoding:?}: {status}");
        assert_eq!(messages, vec![Bytes::from(text.clone())]);
        assert_eq!(flaky_seen(&one_key).len(), 2);
    }
}

async fn heads_of(channel: GrpcChannel) -> String {
    let (_, messages, status) = unary(
        &channel,
        CallStartOptions::new("/raw/EchoHeaders"),
        Bytes::from_static(b"x"),
    )
    .await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    String::from_utf8(messages.concat().to_vec()).expect("the headers as text")
}

#[tokio::test]
async fn the_request_states_what_the_channel_sends_and_accepts() {
    let server = TestServer::start().await;

    let default = heads_of(channel(&server.endpoint)).await;
    assert!(
        default.contains("grpc-accept-encoding=identity"),
        "{default}"
    );
    assert!(!default.contains("grpc-encoding"), "{default}");

    let both = heads_of(compressing(
        &server.endpoint,
        Some(Encoding::Zstd),
        &[Encoding::Gzip],
        |_| {},
    ))
    .await;
    assert!(
        both.contains("grpc-accept-encoding=gzip,identity"),
        "{both}"
    );
    assert!(both.contains("grpc-encoding=zstd"), "{both}");

    let only_sending = heads_of(sending(&server.endpoint, Encoding::Deflate)).await;
    assert!(
        only_sending.contains("grpc-accept-encoding=identity"),
        "{only_sending}"
    );
    assert!(
        only_sending.contains("grpc-encoding=deflate"),
        "{only_sending}"
    );
}

/// The list is advertised in the order given, identity last, and a repeated encoding once at its
/// first place; that is the order a server that takes the first it knows reads as a preference.
#[tokio::test]
async fn the_accepted_encodings_are_advertised_in_order_and_once() {
    let server = TestServer::start().await;
    let accepted = |list: &[Encoding]| heads_of(accepting(&server.endpoint, list));
    use Encoding::*;

    assert!(accepted(&[Zstd, Gzip])
        .await
        .contains("grpc-accept-encoding=zstd,gzip,identity"));
    assert!(accepted(&[Gzip, Zstd])
        .await
        .contains("grpc-accept-encoding=gzip,zstd,identity"));
    assert!(accepted(&ALL)
        .await
        .contains("grpc-accept-encoding=gzip,deflate,zstd,identity"));
    assert!(accepted(&[Deflate, Gzip, Deflate, Gzip, Zstd, Deflate])
        .await
        .contains("grpc-accept-encoding=deflate,gzip,zstd,identity"));
    assert!(accepted(&[])
        .await
        .contains("grpc-accept-encoding=identity"));
}

/// A server that compresses its answers does so only in an encoding the channel advertised, so a
/// channel that advertised none is answered with messages it reads as they are.
#[tokio::test]
async fn a_channel_is_answered_in_the_encodings_it_accepts() {
    let server = TestServer::start().await;
    let text = Bytes::from(b"abc".repeat(1000));

    let mut channels = vec![
        accepting(&server.endpoint, &[]),
        accepting(&server.endpoint, &ALL),
        accepting(&server.endpoint, &[Encoding::Zstd, Encoding::Gzip]),
    ];
    for encoding in ALL {
        channels.push(sending(&server.endpoint, encoding));
        channels.push(accepting(&server.endpoint, &[encoding]));
        channels.push(compressing(
            &server.endpoint,
            Some(encoding),
            &[encoding],
            |_| {},
        ));
    }
    for channel in channels {
        let (_, messages, status) = unary(
            &channel,
            CallStartOptions::new(ECHO_COMPRESSED),
            text.clone(),
        )
        .await;
        assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
        assert_eq!(messages, vec![text.clone()]);
    }
}

#[tokio::test]
async fn a_compressed_answer_is_inflated_when_the_channel_accepts_its_encoding() {
    let server = TestServer::start().await;
    let squeezed = Bytes::from(b"squeezed ".repeat(100));

    for encoding in ALL {
        // Alone, and among the others, the encoding the server uses being any of the list.
        for accepted in [vec![encoding], ALL.to_vec(), vec![Encoding::Gzip, encoding]] {
            let channel = accepting(&server.endpoint, &accepted);

            let (_, messages, status) = unary(
                &channel,
                CallStartOptions::new(format!("/raw/ReplyIn:{}", named(encoding))),
                Bytes::from_static(b"x"),
            )
            .await;
            assert_eq!(
                status.code,
                GrpcStatusCode::Ok,
                "{encoding:?} in {accepted:?}: {status}"
            );
            assert_eq!(messages, vec![squeezed.clone()]);
        }
    }

    let (_, messages, status) = unary(
        &accepting(&server.endpoint, &[Encoding::Gzip]),
        CallStartOptions::new("/raw/GzipReplyLeftPlain"),
        Bytes::from_static(b"x"),
    )
    .await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(messages, vec![Bytes::from_static(b"plain")]);
}

/// The compression document faults a message compressed in an encoding the client does not
/// support, `INTERNAL`. One this channel was not set to accept is such a message, whatever the
/// server is able to send: also one of the three the channel knows, when it was not listed.
#[tokio::test]
async fn a_message_in_an_encoding_the_channel_does_not_accept_ends_the_call_internal() {
    let server = TestServer::start().await;

    let mut cases = vec![
        (channel(&server.endpoint), "/raw/ReplyIn:gzip".to_owned()),
        (
            sending(&server.endpoint, Encoding::Gzip),
            "/raw/ReplyIn:gzip".to_owned(),
        ),
        (
            accepting(&server.endpoint, &[Encoding::Gzip]),
            "/raw/BrotliReply".to_owned(),
        ),
        (
            accepting(&server.endpoint, &ALL),
            "/raw/BrotliReply".to_owned(),
        ),
    ];
    for encoding in ALL {
        let others: Vec<Encoding> = ALL.into_iter().filter(|other| *other != encoding).collect();
        cases.push((
            accepting(&server.endpoint, &others),
            format!("/raw/ReplyIn:{}", named(encoding)),
        ));
    }
    for (channel, method) in cases {
        let (_, messages, status) = unary(
            &channel,
            CallStartOptions::new(method.clone()),
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

    for encoding in ALL {
        let (_, messages, status) = unary(
            &accepting(&server.endpoint, &ALL),
            CallStartOptions::new(format!("/raw/ReplyThatDoesNotInflate:{}", named(encoding))),
            Bytes::from_static(b"x"),
        )
        .await;

        assert_eq!(
            status.code,
            GrpcStatusCode::Internal,
            "{encoding:?}: {status}"
        );
        assert!(status.message.contains("decompress"), "{status}");
        assert!(messages.is_empty());
    }
}

/// The limit is on the message as the application reads it. Eight MiB of zeros is a few KiB on
/// the wire, which no limit on the wire alone would stop.
#[tokio::test]
async fn the_receive_limit_is_on_the_inflated_message() {
    let server = TestServer::start().await;
    let eight_mib = 8 * 1024 * 1024;

    for encoding in ALL {
        let method = format!("/raw/ReplyOfEightMiB:{}", named(encoding));
        let (_, messages, status) = unary(
            &accepting(&server.endpoint, &ALL),
            CallStartOptions::new(method.clone()),
            Bytes::from_static(b"x"),
        )
        .await;
        assert_eq!(
            status.code,
            GrpcStatusCode::ResourceExhausted,
            "{encoding:?}: {status}"
        );
        assert!(status.message.contains("4194304"), "{status}");
        assert!(messages.is_empty());

        let raised = compressing(&server.endpoint, None, &ALL, |config| {
            config.max_recv_message_size = eight_mib
        });
        let (_, messages, status) = unary(
            &raised,
            CallStartOptions::new(method),
            Bytes::from_static(b"x"),
        )
        .await;
        assert_eq!(status.code, GrpcStatusCode::Ok, "{encoding:?}: {status}");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].len(), eight_mib);
    }
}

/// What the engine logs, by the channel's own target: its level and message.
#[derive(Clone, Default)]
struct Logged(std::sync::Arc<std::sync::Mutex<Vec<(tracing::Level, String)>>>);

impl Logged {
    /// Logs from here to the end of the test are the ones this holds, on this thread: the tests
    /// run on a current-thread runtime, so the engine's tasks run here too.
    fn watch(&self) -> tracing::subscriber::DefaultGuard {
        tracing::subscriber::set_default(self.clone())
    }

    fn at(&self, level: tracing::Level) -> Vec<String> {
        let logged = self.0.lock().expect("the log");
        logged
            .iter()
            .filter(|(at, _)| *at == level)
            .map(|(_, text)| text.clone())
            .collect()
    }
}

impl tracing::Subscriber for Logged {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        metadata.target() == "armonik_transport"
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        struct Message(String);
        impl tracing::field::Visit for Message {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message" {
                    self.0 = format!("{value:?}");
                }
            }
        }
        let mut message = Message(String::new());
        event.record(&mut message);
        self.0
            .lock()
            .expect("the log")
            .push((*event.metadata().level(), message.0));
    }

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

async fn echoed(channel: &GrpcChannel, method: &str, text: &[u8]) -> GrpcStatus {
    let (_, messages, status) = unary(
        channel,
        CallStartOptions::new(method),
        Bytes::copy_from_slice(text),
    )
    .await;
    if status.code == GrpcStatusCode::Ok {
        assert_eq!(messages, vec![Bytes::copy_from_slice(text)]);
    }
    status
}

/// A server that does not accept the channel's encoding fails the first call that reaches it, and
/// says what it accepts: the calls after it send as they are, and succeed.
#[tokio::test]
async fn a_server_that_refuses_the_encoding_costs_one_call_and_then_the_channel_stops_compressing()
{
    let server = TestServer::start().await;
    let text = b"abc".repeat(1000);

    for encoding in ALL {
        let channel = sending(&server.endpoint, encoding);

        let first = echoed(&channel, ECHO, &text).await;
        assert_eq!(
            first.code,
            GrpcStatusCode::Unimplemented,
            "{encoding:?}: {first}"
        );

        for _ in 0..3 {
            let next = echoed(&channel, ECHO, &text).await;
            assert_eq!(next.code, GrpcStatusCode::Ok, "{encoding:?}: {next}");
        }
        let said = said_of(&channel, &[&text]).await;
        assert_eq!(said, "encoding=none frames=0:3000", "{encoding:?}");
    }
}

/// The server's list is what decides, not the refusal alone: one that accepts gzip alone tells a
/// channel that sends zstd so, and the channel that sends gzip is not told anything.
#[tokio::test]
async fn the_channel_compresses_while_the_server_lists_its_encoding() {
    let server = TestServer::start().await;
    let text = b"abc".repeat(1000);

    let zstd = sending(&server.endpoint, Encoding::Zstd);
    let first = echoed(&zstd, ECHO_GZIP_ONLY, &text).await;
    assert_eq!(first.code, GrpcStatusCode::Unimplemented, "{first}");
    assert!(first.message.contains("zstd"), "{first}");
    let next = echoed(&zstd, ECHO_GZIP_ONLY, &text).await;
    assert_eq!(next.code, GrpcStatusCode::Ok, "{next}");

    let gzip = sending(&server.endpoint, Encoding::Gzip);
    for _ in 0..3 {
        let status = echoed(&gzip, ECHO_GZIP_ONLY, &text).await;
        assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    }
    let said = said_of(&gzip, &[&text]).await;
    assert!(said.starts_with("encoding=gzip frames=1:"), "{said}");
}

/// The warning is the channel's, once: a server that goes on refusing, or a refusal learned again
/// after the encoding was listed in between, does not say it again.
#[tokio::test]
async fn the_channel_says_once_that_the_server_does_not_list_its_encoding() {
    let server = TestServer::start().await;
    let logged = Logged::default();
    let _watch = logged.watch();
    let text = b"abc".repeat(1000);
    let channel = sending(&server.endpoint, Encoding::Gzip);
    let call = |method: String| {
        let channel = channel.clone();
        async move {
            let (_, _, status) = unary(
                &channel,
                CallStartOptions::new(method),
                Bytes::from_static(b"x"),
            )
            .await;
            assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
        }
    };
    assert!(logged.at(tracing::Level::WARN).is_empty());

    let first = echoed(&channel, ECHO, &text).await;
    assert_eq!(first.code, GrpcStatusCode::Unimplemented, "{first}");
    let warnings = logged.at(tracing::Level::WARN);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("does not list"), "{warnings:?}");

    call("/raw/Accepts:identity".to_owned()).await;
    call("/raw/Accepts:deflate,identity".to_owned()).await;
    assert_eq!(logged.at(tracing::Level::WARN).len(), 1, "still the one");

    call("/raw/Accepts:gzip,identity".to_owned()).await;
    assert!(
        logged
            .at(tracing::Level::DEBUG)
            .iter()
            .any(|said| said.contains("compress their messages again")),
        "the resume is said"
    );
    assert_eq!(
        logged.at(tracing::Level::WARN).len(),
        1,
        "and is no warning"
    );
    call("/raw/Accepts:identity".to_owned()).await;
    assert_eq!(
        logged.at(tracing::Level::WARN).len(),
        1,
        "a second refusal on the channel is not a second warning"
    );
}

/// A later answer that lists the encoding has the channel compress again, as a server that was
/// upgraded in between would; one that does not list it, or says nothing, leaves it as it was.
#[tokio::test]
async fn the_channel_compresses_again_once_a_response_lists_the_encoding() {
    let server = TestServer::start().await;
    let text = b"abc".repeat(1000);
    let channel = sending(&server.endpoint, Encoding::Gzip);
    let compressing = || async { said_of(&channel, &[&text]).await };
    let hears = |method: &'static str| {
        let channel = channel.clone();
        async move {
            let (_, _, status) = unary(
                &channel,
                CallStartOptions::new(method),
                Bytes::from_static(b"x"),
            )
            .await;
            assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
        }
    };

    assert!(compressing().await.starts_with("encoding=gzip"));

    let refused = echoed(&channel, ECHO, &text).await;
    assert_eq!(refused.code, GrpcStatusCode::Unimplemented, "{refused}");
    assert_eq!(compressing().await, "encoding=none frames=0:3000");

    // A head that lists other encodings, or none at all, is no word on gzip.
    hears("/raw/Accepts:deflate,zstd,identity").await;
    hears("/raw/EchoHeaders").await;
    assert_eq!(compressing().await, "encoding=none frames=0:3000");

    hears("/raw/Accepts:zstd,GZIP,identity").await;
    assert!(compressing().await.starts_with("encoding=gzip frames=1:"));

    hears("/raw/Accepts:identity").await;
    assert_eq!(compressing().await, "encoding=none frames=0:3000");
}

/// The head of a response that is not gRPC says nothing of what the server accepts.
#[tokio::test]
async fn a_response_that_is_not_grpc_teaches_the_channel_nothing() {
    let server = TestServer::start().await;
    let channel = sending(&server.endpoint, Encoding::Gzip);

    let (_, _, status) = unary(
        &channel,
        CallStartOptions::new("/raw/NotGrpcThatLists"),
        Bytes::from_static(b"x"),
    )
    .await;
    assert_ne!(status.code, GrpcStatusCode::Ok, "{status}");

    let said = said_of(&channel, &[&b"abc".repeat(1000)]).await;
    assert!(said.starts_with("encoding=gzip frames=1:"), "{said}");
}

/// A call has its messages compressed and its header says so from the moment it starts: what the
/// channel learns while it runs changes the calls that start after it.
#[tokio::test]
async fn a_call_keeps_the_encoding_it_started_with() {
    let server = TestServer::start().await;
    let channel = sending(&server.endpoint, Encoding::Gzip);
    let text = b"abc".repeat(1000);

    let (mut send, mut recv, _control) = channel
        .start_call(CallStartOptions::new(FRAMES))
        .expect("the call starts")
        .split();
    send.send_message(Bytes::from(text.clone()))
        .await
        .expect("sent");

    let refused = echoed(&channel, ECHO, &text).await;
    assert_eq!(refused.code, GrpcStatusCode::Unimplemented, "{refused}");

    send.send_message(Bytes::from(text.clone()))
        .await
        .expect("sent");
    let _ = send.end_send().await;
    let (_, heard, status) = read_to_terminal(&mut recv).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    let said = String::from_utf8(heard.concat()).expect("text");
    assert!(said.starts_with("encoding=gzip frames=1:"), "{said}");
    assert_eq!(said.matches(",1:").count(), 1, "both compressed: {said}");
}
