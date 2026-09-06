mod common;

use armonik_transport::grpc::{CallStartOptions, GrpcStatusCode, RecvResult};
use bytes::Bytes;
use common::echo::*;

/// The messages a client streaming call sends, and what the server answers having read them.
const SENT: [&str; 3] = ["one", "two", "three"];

#[tokio::test]
async fn every_message_of_a_client_stream_reaches_the_server_before_its_one_reply() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    let (mut send, mut recv, _control) = channel
        .start_call(CallStartOptions::new(COLLECT))
        .expect("the call starts")
        .split();

    for text in SENT {
        send.send_message(Bytes::from_static(text.as_bytes()))
            .await
            .expect("a send window of one still takes every message in turn");
    }
    send.end_send().await.expect("the half-close");

    let (_head, messages, status) = read_to_terminal(&mut recv).await;

    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(
        messages,
        vec![Bytes::from(format!("{}:{}", SENT.len(), SENT.join(",")))],
        "the server saw every message, in order, and answered once"
    );
}

/// Each message answered before the next is sent, which a single window has to allow.
#[tokio::test]
async fn a_bidi_call_answers_each_message_before_the_next_is_sent() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    let (mut send, mut recv, _control) = channel
        .start_call(CallStartOptions::new(CHAT))
        .expect("the call starts")
        .split();

    for text in SENT {
        send.send_message(Bytes::from_static(text.as_bytes()))
            .await
            .expect("the window has room");

        match recv.next_message().await.expect("an answer") {
            RecvResult::Message(answered) => {
                assert_eq!(answered.data, Bytes::from_static(text.as_bytes()))
            }
            other => panic!("{other:?}"),
        }
    }

    send.end_send().await.expect("the half-close");

    let terminal = recv.next_message().await.expect("a terminal");
    match terminal {
        RecvResult::End(status) => assert_eq!(status.code, GrpcStatusCode::Ok, "{status}"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn every_message_of_a_server_stream_comes_back_in_order() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    let (mut send, mut recv, _control) = channel
        .start_call(CallStartOptions::new(FAN))
        .expect("the call starts")
        .split();
    send.send_message(Bytes::from(SENT.join(",")))
        .await
        .expect("the one request");
    send.end_send().await.expect("the half-close");

    let (_head, messages, status) = read_to_terminal(&mut recv).await;

    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(
        messages,
        SENT.iter()
            .map(|text| Bytes::from_static(text.as_bytes()))
            .collect::<Vec<_>>(),
        "one message per part, in order"
    );
}

#[tokio::test]
async fn a_server_stream_that_answers_nothing_still_reaches_its_terminal() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    let (mut send, mut recv, _control) = channel
        .start_call(CallStartOptions::new(FAN))
        .expect("the call starts")
        .split();
    send.send_message(Bytes::new()).await.expect("the request");
    send.end_send().await.expect("the half-close");

    let (_head, messages, status) = read_to_terminal(&mut recv).await;

    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert!(messages.is_empty(), "{messages:?}");
}

#[tokio::test]
async fn a_client_stream_that_sends_nothing_still_reaches_its_reply() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    let (send, mut recv, _control) = channel
        .start_call(CallStartOptions::new(COLLECT))
        .expect("the call starts")
        .split();

    send.end_send().await.expect("the half-close");

    let (_head, messages, status) = read_to_terminal(&mut recv).await;

    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(messages, vec![Bytes::from_static(b"0:")]);
}
