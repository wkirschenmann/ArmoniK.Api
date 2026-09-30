//! A call's driver that panics, held to the promise every call keeps: it ends with a status.

// The fixture serves every test binary; which parts this one reaches is not a fact about it.
#[allow(dead_code)]
mod common;

use std::sync::Arc;
use std::time::Duration;

use armonik_transport::grpc::{CallStartOptions, GrpcStatusCode, RecvResult};
use armonik_transport::hooks;
use common::echo::*;

#[tokio::test]
async fn a_driver_that_panics_ends_its_call_with_an_internal_status() {
    hooks::in_driver(Some(Arc::new(|| panic!("the driver panics"))));
    let _unhook = Unhook;

    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);
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
        Ok(RecvResult::End(status)) => {
            assert_eq!(status.code, GrpcStatusCode::Internal, "{status:?}")
        }
        other => panic!("the call ended without a status: {other:?}"),
    }
}

/// Takes the hook away however the test ends: it is process-wide.
struct Unhook;

impl Drop for Unhook {
    fn drop(&mut self) {
        hooks::in_driver(None);
    }
}
