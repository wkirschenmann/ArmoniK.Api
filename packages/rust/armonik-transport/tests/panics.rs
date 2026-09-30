//! Engine tasks that panic, held to the promise every call keeps: it ends, and with a status.
//!
//! One at a time, because a hook is process-wide and would reach the other test's calls.

// The fixture serves every test binary; which parts this one reaches is not a fact about it.
#[allow(dead_code)]
mod common;

use std::sync::Arc;
use std::time::Duration;

use armonik_transport::grpc::{
    CallStartOptions, GrpcChannel, GrpcStatus, GrpcStatusCode, RecvResult,
};
use armonik_transport::hooks::{self, Hook};
use bytes::Bytes;
use common::echo::*;
use serial_test::serial;

#[tokio::test]
#[serial]
async fn a_driver_that_panics_ends_its_call_with_an_internal_status() {
    hooks::in_driver(Some(panicking("the driver panics")));
    let _unhook = Unhook(hooks::in_driver);

    let server = TestServer::start().await;
    let status = ended(&channel(&server.endpoint)).await;

    assert_eq!(status.code, GrpcStatusCode::Internal, "{status:?}");
}

/// A dial that panics fails the calls waiting on it rather than leaving them waiting, and leaves
/// the channel free to dial again for the next.
#[tokio::test]
#[serial]
async fn a_dial_that_panics_fails_its_calls_and_the_next_call_dials_again() {
    hooks::in_dial(Some(panicking("the dial panics")));
    let unhook = Unhook(hooks::in_dial);

    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);
    let refused = ended(&channel).await;
    assert_eq!(refused.code, GrpcStatusCode::Unavailable, "{refused:?}");

    drop(unhook);
    let (_, _, served) = unary(
        &channel,
        CallStartOptions::new(ECHO),
        Bytes::from_static(b"again"),
    )
    .await;
    assert_eq!(served.code, GrpcStatusCode::Ok, "{served:?}");
}

fn panicking(why: &'static str) -> Hook {
    Arc::new(move || panic!("{why}"))
}

/// The status an ECHO call ends with, however it ends: a call that ends without one fails here,
/// and so does one that does not end.
async fn ended(channel: &GrpcChannel) -> GrpcStatus {
    let (send, mut recv, _control) = channel
        .start_call(CallStartOptions::new(ECHO))
        .expect("the call starts")
        .split();
    let _ = send.end_send().await;

    let ended = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match recv.next_message().await {
                Ok(RecvResult::Message(_)) => continue,
                other => break other,
            }
        }
    })
    .await
    .expect("the call ends");

    match ended {
        Ok(RecvResult::End(status)) => status,
        other => panic!("the call ended without a status: {other:?}"),
    }
}

/// Takes a hook away however the test ends.
struct Unhook(fn(Option<Hook>));

impl Drop for Unhook {
    fn drop(&mut self) {
        (self.0)(None);
    }
}
