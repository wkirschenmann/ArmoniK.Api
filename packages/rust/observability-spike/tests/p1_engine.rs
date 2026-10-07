//! Points 1 and 2 against the real engine: a channel on a runtime's thread, h2 and the engine's own
//! events reaching that runtime's callback and no other.

mod engine_support;

use std::time::Duration;

use armonik_transport::grpc::{CallStartOptions, GrpcChannel, GrpcChannelConfig, GrpcStatusCode};
use armonik_transport::http2::TransportConfig;
use bytes::Bytes;
use engine_support::echo::{read_to_terminal, TestServer, ECHO};
use http::Uri;
use observability_spike::runtime::{ChannelThread, Front, ObsRuntime};
use observability_spike::testkit::{collect, Collector};

fn channel_on(thread: &ChannelThread, endpoint: &str) -> GrpcChannel {
    let uri = Uri::try_from(endpoint).unwrap();
    let mut config = GrpcChannelConfig::new(TransportConfig::new(uri));
    config.transport.connect_timeout = Duration::from_secs(5);
    GrpcChannel::new(config, thread.handle.clone()).expect("a channel")
}

/// One unary call, driven from the calling thread under the runtime's scope.
fn call_once(runtime: &ObsRuntime, channel: &GrpcChannel) {
    let _inside = runtime.scope();
    runtime.tokio.block_on(async {
        let (mut send, mut recv, _control) = channel
            .start_call(CallStartOptions::new(ECHO))
            .expect("the call starts")
            .split();
        let _ = send.send_message(Bytes::from_static(b"hello")).await;
        let _ = send.end_send().await;
        let (_, _, status) = read_to_terminal(&mut recv).await;
        assert_eq!(status.code, GrpcStatusCode::Ok);
    });
}

fn count_by_target(log: &Collector) -> std::collections::BTreeMap<String, usize> {
    let mut by = std::collections::BTreeMap::new();
    for record in log.records.lock().unwrap().iter() {
        let root = record.target.split("::").next().unwrap().to_owned();
        *by.entry(root).or_insert(0) += 1;
    }
    by
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_engines_and_h2s_events_reach_their_runtime_alone() {
    let server_a = TestServer::start().await;
    let server_b = TestServer::start().await;
    let endpoint_a = server_a.endpoint.clone();
    let endpoint_b = server_b.endpoint.clone();

    let outcome = tokio::task::spawn_blocking(move || {
        let a = ObsRuntime::new(Front::Layered);
        let b = ObsRuntime::new(Front::Layered);
        let log_a = Box::new(Collector::default());
        let log_b = Box::new(Collector::default());
        // A asks for h2's frames and the engine's debug; B keeps the default.
        a.obs
            .set_log_callback(collect, log_a.ctx(), "info,armonik_transport=debug,h2=trace,hyper=debug")
            .unwrap();
        b.obs.set_log_callback(collect, log_b.ctx(), "").unwrap();
        let thread_a = a.start_channel_thread();
        let thread_b = b.start_channel_thread();
        let channel_a = channel_on(&thread_a, &endpoint_a);
        let channel_b = channel_on(&thread_b, &endpoint_b);

        call_once(&a, &channel_a);
        call_once(&b, &channel_b);
        drop(channel_a);
        drop(channel_b);
        drop(thread_a);
        drop(thread_b);
        (count_by_target(&log_a), count_by_target(&log_b))
    })
    .await
    .unwrap();
    println!("runtime A (h2=trace): {:?}", outcome.0);
    println!("runtime B (default):  {:?}", outcome.1);
    assert!(outcome.0.get("h2").copied().unwrap_or(0) > 0, "A asked for h2");
    assert!(!outcome.1.contains_key("h2"), "B did not");
}

/// The server's own threads belong to no runtime: their h2 events go nowhere.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_changed_filter_brings_h2_back_for_the_next_connection() {
    let server = TestServer::start().await;
    let endpoint = server.endpoint.clone();
    let counts = tokio::task::spawn_blocking(move || {
        let a = ObsRuntime::new(Front::Layered);
        let log = Box::new(Collector::default());
        a.obs.set_log_callback(collect, log.ctx(), "").unwrap();
        let thread = a.start_channel_thread();
        let channel = channel_on(&thread, &endpoint);
        call_once(&a, &channel);
        let before = count_by_target(&log);
        a.obs.set_filter("info,h2=trace").unwrap();
        call_once(&a, &channel);
        let after = count_by_target(&log);
        (before, after)
    })
    .await
    .unwrap();
    println!("before set_filter: {:?}\nafter set_filter:  {:?}", counts.0, counts.1);
    assert!(!counts.0.contains_key("h2"));
    assert!(counts.1.get("h2").copied().unwrap_or(0) > 0);
}
