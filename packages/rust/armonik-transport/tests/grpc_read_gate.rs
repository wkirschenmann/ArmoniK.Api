//! A read gate decides when a call reads its next message: the engine waits on it before each
//! read, and the call's own ends still end a call it holds.

mod common;

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use armonik_transport::grpc::{
    CallControl, CallStartOptions, Deadline, GrpcChannel, GrpcStatus, GrpcStatusCode, ReadGate,
    RecvHalf, RecvResult,
};
use bytes::Bytes;
use common::echo::*;

/// Admits every read, or none, and counts the reads it was asked for.
#[derive(Debug, Default)]
struct Gate {
    shut: bool,
    asked: AtomicUsize,
    turns: AtomicUsize,
}

impl ReadGate for Gate {
    fn admitted(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if self.shut {
                std::future::pending::<()>().await;
            }
        })
    }

    fn turn(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        self.turns.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {})
    }
}

/// Answers with three messages, then OK.
async fn start(
    channel: &GrpcChannel,
    gate: &Arc<Gate>,
    deadline: Option<Duration>,
) -> (RecvHalf, CallControl) {
    let mut options = CallStartOptions::new("/raw/Sized");
    options
        .metadata
        .append_ascii("x-sizes", "10,10,10")
        .expect("valid metadata");
    options.deadline = deadline.map(Deadline::Timeout);
    options.read_gate = Some(Arc::clone(gate) as _);

    let (send, recv, control) = channel
        .start_call(options)
        .expect("the call starts")
        .split();
    send.end_send().await.expect("the half-close");
    (recv, control)
}

async fn to_the_end(recv: &mut RecvHalf) -> (Vec<Bytes>, GrpcStatus) {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut messages = Vec::new();
        loop {
            match recv.next_message().await.expect("a message or a status") {
                RecvResult::Message(message) => messages.push(message.data),
                RecvResult::End(status) => return (messages, status),
            }
        }
    })
    .await
    .expect("the call ends")
}

async fn sized(
    channel: &GrpcChannel,
    gate: &Arc<Gate>,
    deadline: Option<Duration>,
) -> (Vec<Bytes>, GrpcStatus) {
    let (mut recv, _control) = start(channel, gate, deadline).await;
    to_the_end(&mut recv).await
}

#[tokio::test]
async fn the_gate_is_asked_before_each_read_the_status_included() {
    let server = TestServer::start().await;
    let gate = Arc::new(Gate::default());

    let (messages, status) = sized(&channel(&server.endpoint), &gate, None).await;

    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(messages.len(), 3);
    assert_eq!(
        gate.asked.load(Ordering::SeqCst),
        4,
        "three messages, and the read that found the trailers"
    );
}

/// On a call that declared one response, the read after the message asks for the turn alone: only
/// the status can follow, and it is not what the gate's admission holds back.
#[tokio::test]
async fn a_one_response_call_asks_only_the_turn_for_its_status() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);
    let gate = Arc::new(Gate::default());

    let mut options = CallStartOptions::new("/raw/Sized");
    options
        .metadata
        .append_ascii("x-sizes", "10")
        .expect("valid metadata");
    options.read_gate = Some(Arc::clone(&gate) as _);
    options.one_response = true;
    let (send, mut recv, _control) = channel
        .start_call(options)
        .expect("the call starts")
        .split();
    send.end_send().await.expect("the half-close");

    let (messages, status) = to_the_end(&mut recv).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(messages.len(), 1);
    assert_eq!(gate.asked.load(Ordering::SeqCst), 1, "the message's read");
    assert_eq!(gate.turns.load(Ordering::SeqCst), 1, "the status's");
}

#[tokio::test]
async fn a_call_the_gate_holds_reads_nothing_and_still_ends_at_its_deadline() {
    let server = TestServer::start().await;
    let gate = Arc::new(Gate {
        shut: true,
        ..Gate::default()
    });

    let (messages, status) = sized(
        &channel(&server.endpoint),
        &gate,
        Some(Duration::from_millis(300)),
    )
    .await;

    assert_eq!(status.code, GrpcStatusCode::DeadlineExceeded, "{status}");
    assert!(messages.is_empty(), "nothing was read past the gate");
    assert_eq!(gate.asked.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_call_the_gate_holds_ends_when_it_is_cancelled_or_its_channel_closes() {
    let server = TestServer::start().await;
    for closing in [false, true] {
        let channel = channel(&server.endpoint);
        let gate = Arc::new(Gate {
            shut: true,
            ..Gate::default()
        });
        let (mut recv, control) = start(&channel, &gate, None).await;

        // Held at the gate, past the head.
        tokio::time::timeout(Duration::from_secs(10), async {
            while gate.asked.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the call reaches the gate");
        if closing {
            channel.close();
        } else {
            control.cancel();
        }

        let (messages, status) = to_the_end(&mut recv).await;
        assert_eq!(
            status.code,
            GrpcStatusCode::Cancelled,
            "{closing}: {status}"
        );
        assert!(messages.is_empty(), "nothing was read past the gate");
    }
}
