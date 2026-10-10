//! A build without the `metrics` feature counts nothing, and says so.

#![cfg(not(feature = "metrics"))]

mod common;

use armonik_transport::grpc::{CallStartOptions, GrpcStatusCode};
use armonik_transport::metrics::{Metrics, Stats};
use bytes::Bytes;
use common::echo::*;

#[tokio::test]
async fn a_channel_answers_an_empty_stats_after_its_calls() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    let (_, _, status) = unary(
        &channel,
        CallStartOptions::new(ECHO),
        Bytes::from_static(b"hello"),
    )
    .await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");

    let stats = channel.stats();
    assert!(!stats.counting);
    assert_eq!(stats, Stats::default());
}

#[test]
fn a_registry_answers_an_empty_stats() {
    assert_eq!(Metrics::new().stats(), Stats::default());
}
