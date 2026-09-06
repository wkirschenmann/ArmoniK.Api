mod common;

use armonik_transport::grpc::{
    CallError, CallStartOptions, ChannelError, GrpcChannelConfig, GrpcStatusCode, MetadataValue,
};
use armonik_transport::http2::{TransportConfig, TransportErrorKind};
use bytes::Bytes;
use common::echo::*;
use http::Uri;

#[tokio::test]
async fn a_unary_call_reaches_a_grpc_server_and_comes_back() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    let mut options = CallStartOptions::new(ECHO);
    options
        .metadata
        .append_ascii("x-request", "ping")
        .expect("a plain metadata entry");

    let (head, messages, status) = unary(&channel, options, Bytes::from_static(b"hello")).await;

    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(messages, vec![Bytes::from_static(b"hello")]);
    assert_eq!(
        head.get("x-echoed"),
        Some(&MetadataValue::Ascii("ping".to_owned())),
        "the request metadata reached the server and its answer came back"
    );
}

#[tokio::test]
async fn an_empty_message_is_a_message_and_not_an_absence() {
    let (_, messages, status) = call_on(ECHO, Bytes::new()).await;

    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(messages, vec![Bytes::new()]);
}

#[tokio::test]
async fn a_message_larger_than_one_http2_frame_survives_the_round_trip() {
    const SIZE: usize = 256 * 1024;

    let (_, messages, status) = call_on(ECHO, Bytes::from(vec![0x5a; SIZE])).await;

    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].len(), SIZE);
    assert!(messages[0].iter().all(|byte| *byte == 0x5a));
}

#[tokio::test]
async fn a_binary_metadata_entry_crosses_the_wire_as_bytes() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    let mut options = CallStartOptions::new(ECHO);
    options
        .metadata
        .append_binary("x-request-bin", Bytes::from_static(&[0, 1, 2, 0xff]))
        .expect("a binary entry under a binary key");

    let (head, _, status) = unary(&channel, options, Bytes::from_static(b"x")).await;

    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(
        head.get("x-echoed-bin"),
        Some(&MetadataValue::Binary(Bytes::from_static(&[0, 1, 2, 0xff])))
    );
}

#[tokio::test]
async fn the_request_carries_the_headers_grpc_asks_for() {
    let (_, messages, status) = call_on("/raw/EchoHeaders", Bytes::from_static(b"x")).await;

    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    let seen = String::from_utf8(messages.concat().to_vec()).expect("the headers as text");
    for expected in [
        "content-type=application/grpc",
        "te=trailers",
        "grpc-accept-encoding=identity",
        "user-agent=armonik-transport/",
    ] {
        assert!(seen.contains(expected), "{expected} missing from {seen}");
    }
}

#[tokio::test]
async fn a_method_the_server_refuses_comes_back_as_its_status_and_its_trailers() {
    let (head, messages, status) = call_on(FAIL, Bytes::from_static(b"x")).await;

    assert_eq!(status.code, GrpcStatusCode::PermissionDenied);
    assert_eq!(status.message, "not for you");
    assert_eq!(
        status.trailing_metadata.get("x-reason"),
        Some(&MetadataValue::Ascii("policy".to_owned()))
    );
    assert!(messages.is_empty());
    assert!(head.is_empty(), "{head:?}");
}

#[tokio::test]
async fn a_method_the_server_does_not_have_is_unimplemented() {
    let (_, _, status) = call_on(
        "/armonik_transport.test.Echo/Absent",
        Bytes::from_static(b"x"),
    )
    .await;

    assert_eq!(status.code, GrpcStatusCode::Unimplemented, "{status}");
    assert_eq!(status.message, "no such method");
}

#[tokio::test]
async fn calls_on_one_channel_share_one_session() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    let mut running = Vec::new();
    for _ in 0..8 {
        let channel = channel.clone();
        running.push(tokio::spawn(async move {
            unary(
                &channel,
                CallStartOptions::new(ECHO),
                Bytes::from_static(b"concurrent"),
            )
            .await
        }));
    }

    for call in running {
        let (_, messages, status) = call.await.expect("the call ran to its end");
        assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
        assert_eq!(messages, vec![Bytes::from_static(b"concurrent")]);
    }

    assert_eq!(
        server.connections(),
        1,
        "eight calls, one HTTP/2 connection"
    );
}

#[tokio::test]
async fn nothing_can_be_sent_once_the_call_has_reached_its_terminal() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    let (mut send, mut recv, _control) = channel
        .start_call(CallStartOptions::new("/raw/HeadThenError"))
        .expect("the call starts")
        .split();

    let (_, _, status) = read_to_terminal(&mut recv).await;
    assert_eq!(status.code, GrpcStatusCode::ResourceExhausted, "{status}");

    assert_eq!(
        send.send_message(Bytes::from_static(b"too late")).await,
        Err(CallError::Ended),
        "the abstract model guards a send on a call with no status yet"
    );
}

#[tokio::test]
async fn dropping_the_reading_half_ends_the_call() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    let (mut send, recv, _control) = channel
        .start_call(CallStartOptions::new(SLOW))
        .expect("the call starts")
        .split();
    send.send_message(Bytes::from_static(b"x"))
        .await
        .expect("the message is accepted");

    drop(recv);

    assert_eq!(
        send.send_message(Bytes::from_static(b"more")).await,
        Err(CallError::Ended)
    );
}

#[tokio::test]
async fn a_cancelled_call_ends_as_cancelled_without_waiting_for_the_server() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    let (mut send, mut recv, control) = channel
        .start_call(CallStartOptions::new(SLOW))
        .expect("the call starts")
        .split();
    send.send_message(Bytes::from_static(b"x"))
        .await
        .expect("the message is accepted");
    send.end_send().await.expect("the request half-closes");

    control.cancel();
    control.cancel();

    ends_cancelled(
        &mut recv,
        "cancelling does not wait for the server's own answer",
    )
    .await;
    assert_eq!(
        recv.next_message().await,
        Err(CallError::Ended),
        "nothing follows the terminal"
    );
}

#[tokio::test]
async fn closing_a_channel_refuses_new_calls_and_ends_the_ones_under_way() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    let (mut send, mut recv, _control) = channel
        .start_call(CallStartOptions::new(SLOW))
        .expect("the call starts")
        .split();
    send.send_message(Bytes::from_static(b"x"))
        .await
        .expect("the message is accepted");
    send.end_send().await.expect("the request half-closes");

    channel.close();

    assert_eq!(
        channel.start_call(CallStartOptions::new(ECHO)).err(),
        Some(ChannelError::Closed)
    );

    ends_cancelled(&mut recv, "closing ends the calls under way").await;
}

#[tokio::test]
async fn an_endpoint_nobody_answers_ends_the_call_rather_than_failing_to_start_it() {
    let endpoint = closed_port().await;
    let (_, messages, status) = unary(
        &channel(&endpoint),
        CallStartOptions::new(ECHO),
        Bytes::from_static(b"x"),
    )
    .await;

    assert_eq!(status.code, GrpcStatusCode::Unavailable, "{status}");
    assert!(messages.is_empty());
}

#[tokio::test]
async fn callers_waiting_on_one_dial_share_its_failure() {
    const BLACK_HOLE: &str = "http://192.0.2.1:9";
    let budget = std::time::Duration::from_millis(250);

    let mut transport = TransportConfig::new(BLACK_HOLE.parse::<Uri>().expect("a URI"));
    transport.connect_timeout = budget;
    let mut config = GrpcChannelConfig::new(transport);
    config.user_agent = Some("test".to_owned());
    let channel = common::echo::channel_with(config).expect("a plain endpoint and default options");

    let alone = std::time::Instant::now();
    channel
        .connect()
        .await
        .expect_err("nothing is reachable there");
    let alone = alone.elapsed();
    if alone < budget {
        eprintln!("skipped: {BLACK_HOLE} answered in {alone:?}, so it is routed here");
        return;
    }

    let together = std::time::Instant::now();
    let waiting: Vec<_> = (0..8)
        .map(|_| {
            let channel = channel.clone();
            tokio::spawn(async move { channel.connect().await })
        })
        .collect();
    for attempt in waiting {
        attempt
            .await
            .expect("the task ran")
            .expect_err("nothing is reachable there");
    }
    let together = together.elapsed();

    assert!(
        together < budget * 3,
        "eight waiters took {together:?} against a budget of {budget:?}, so they dialled one \
         after another instead of sharing the attempt they waited on"
    );
}

#[tokio::test]
async fn connecting_up_front_reports_what_a_call_would_have_reported() {
    let server = TestServer::start().await;
    channel(&server.endpoint)
        .connect()
        .await
        .expect("the test server is listening");

    let error = channel(&closed_port().await)
        .connect()
        .await
        .expect_err("nothing is listening there");
    match error {
        ChannelError::Transport { source } => {
            assert_eq!(source.kind(), TransportErrorKind::Connect, "{source}")
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_closed_channel_opens_no_session() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    channel.close();

    assert_eq!(channel.connect().await, Err(ChannelError::Closed));
    assert_eq!(server.connections(), 0);
}

#[tokio::test]
async fn an_https_endpoint_is_refused_rather_than_dialled_in_the_clear() {
    let config = GrpcChannelConfig::new(TransportConfig::new(Uri::from_static(
        "https://127.0.0.1:443",
    )));

    let error = common::echo::channel_with(config)
        .expect_err("this connector speaks plain HTTP and says so");
    assert!(error.to_string().contains("https://"), "{error}");
}

#[tokio::test]
async fn a_send_window_of_nothing_is_refused() {
    let mut config =
        GrpcChannelConfig::new(TransportConfig::new(Uri::from_static("http://127.0.0.1:1")));
    config.max_sends_in_flight = 0;

    common::echo::channel_with(config).expect_err("a window of zero would let a call send nothing");
}

#[tokio::test]
async fn an_http_error_page_is_reported_as_the_code_grpc_gives_it() {
    let (_, _, status) = call_on("/raw/NotFound", Bytes::from_static(b"x")).await;

    assert_eq!(status.code, GrpcStatusCode::Unimplemented, "{status}");
    assert!(status.message.contains("HTTP 404"), "{status}");
}

#[tokio::test]
async fn a_status_the_peer_states_stands_even_behind_an_http_error() {
    let (_, _, status) = call_on("/raw/StatusBehindError", Bytes::from_static(b"x")).await;

    assert_eq!(status.code, GrpcStatusCode::ResourceExhausted, "{status}");
    assert_eq!(status.message, "no room left");
}

#[tokio::test]
async fn a_two_hundred_that_is_not_grpc_is_an_internal_failure() {
    let (_, _, status) = call_on("/raw/NotGrpc", Bytes::from_static(b"x")).await;

    assert_eq!(status.code, GrpcStatusCode::Internal, "{status}");
    assert!(status.message.contains("content type"), "{status}");
}

#[tokio::test]
async fn a_compressed_message_ends_the_call_rather_than_being_read_as_bytes() {
    let (_, messages, status) = call_on("/raw/Compressed", Bytes::from_static(b"x")).await;

    assert_eq!(status.code, GrpcStatusCode::Internal, "{status}");
    assert!(status.message.contains("compressed"), "{status}");
    assert!(messages.is_empty());
}

#[tokio::test]
async fn a_message_past_the_maximum_ends_the_call_rather_than_being_held() {
    let (_, messages, status) = call_on("/raw/TooBig", Bytes::from_static(b"x")).await;

    assert_eq!(status.code, GrpcStatusCode::ResourceExhausted, "{status}");
    assert!(status.message.contains("67108864"), "{status}");
    assert!(messages.is_empty());
}

#[tokio::test]
async fn a_reply_past_the_maximum_is_refused_and_a_raised_maximum_carries_it() {
    const SIZE: usize = 5 * 1024 * 1024;

    let server = TestServer::start().await;
    let payload = Bytes::from(vec![0x27; SIZE]);

    let (_, _, status) = unary(
        &channel(&server.endpoint),
        CallStartOptions::new(ECHO),
        payload.clone(),
    )
    .await;
    assert_eq!(status.code, GrpcStatusCode::ResourceExhausted, "{status}");

    let uri = Uri::try_from(server.endpoint.as_str()).expect("the test server's endpoint");
    let mut config = GrpcChannelConfig::new(TransportConfig::new(uri));
    config.max_recv_message_size = 8 * 1024 * 1024;
    let roomy = common::echo::channel_with(config).expect("a plain endpoint");

    let (_, messages, status) = unary(&roomy, CallStartOptions::new(ECHO), payload).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].len(), SIZE);
    assert!(messages[0].iter().all(|byte| *byte == 0x27));
}

#[tokio::test]
async fn a_status_behind_a_response_head_is_read_off_the_trailers() {
    let (head, messages, status) = call_on("/raw/HeadThenError", Bytes::from_static(b"x")).await;

    assert_eq!(status.code, GrpcStatusCode::ResourceExhausted, "{status}");
    assert_eq!(status.message, "no room left");
    assert_eq!(messages, vec![Bytes::from_static(b"partial")]);
    assert_eq!(
        head.get("x-head"),
        Some(&MetadataValue::Ascii("present".to_owned()))
    );
}

#[tokio::test]
async fn a_stream_that_ends_without_a_status_is_an_internal_failure() {
    let (_, messages, status) = call_on("/raw/NoTrailers", Bytes::from_static(b"x")).await;

    assert_eq!(status.code, GrpcStatusCode::Internal, "{status}");
    assert!(status.message.contains("grpc-status"), "{status}");
    assert_eq!(messages, vec![Bytes::from_static(b"orphan")]);
}

/// A reset is not a lost connection, and the code says which.
///
/// hyper's server resets with INTERNAL_ERROR when a response body errors. What this test really
/// guards is the step before the table: the reason is read by downcasting hyper's error source to
/// `h2::Error`, which finds nothing at all if hyper ever links an `h2` this crate does not name -
/// and finding nothing is UNAVAILABLE, silently, for every reset there is.
#[tokio::test]
async fn a_stream_the_peer_resets_carries_the_reason_it_was_reset_with() {
    let (_, _messages, status) = call_on("/raw/ResetsMidBody", Bytes::from_static(b"x")).await;

    // Both halves, because either alone would pass for the wrong reason. The message carries the
    // h2 error, which is what tells this apart from a stream that merely ended without a status -
    // that one is Internal too. And the code is Internal rather than Unavailable, which is the
    // reason being read at all: every reset was UNAVAILABLE before, whatever it said.
    assert!(status.message.contains("http2 error"), "{status}");
    assert_eq!(status.code, GrpcStatusCode::Internal, "{status}");
}

#[tokio::test]
async fn a_call_outlives_the_channel_handle_it_was_started_on() {
    let server = TestServer::start().await;

    // The channel is a temporary: it is dropped at the end of this statement, and the driver is
    // then the only thing holding the call together. Dropping a handle is not closing a channel.
    let (mut send, mut recv, _control) = channel(&server.endpoint)
        .start_call(CallStartOptions::new(ECHO))
        .expect("the call starts")
        .split();

    let _ = send.send_message(Bytes::from_static(b"x")).await;
    let _ = send.end_send().await;

    let (_, messages, status) = read_to_terminal(&mut recv).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(messages, vec![Bytes::from_static(b"x")]);
}
