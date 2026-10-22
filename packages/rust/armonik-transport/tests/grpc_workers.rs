//! A channel on a pool of workers: each connection runs on one worker, with every call it carries,
//! and the pool gives new connections to the workers by its rule.

mod common;

use std::collections::BTreeMap;

use armonik_transport::grpc::{
    CallStartOptions, GrpcChannel, GrpcChannelConfig, GrpcStatus, GrpcStatusCode, ResponseHead,
    ResponseSink, Workers,
};
use armonik_transport::http2::TransportConfig;
use armonik_transport::options::WorkerAssignment;
use bytes::Bytes;
use common::echo::*;
use http::Uri;
use tokio::sync::{oneshot, watch};

/// A pool of `count` workers, each a current-thread runtime on a thread named `w-<index>`, which
/// stop when the returned sender is dropped.
fn pool(count: usize, assignment: WorkerAssignment) -> (Workers, watch::Sender<()>) {
    let stop = watch::channel(()).0;
    let stopped = stop.subscribe();
    let workers = Workers::new(count, assignment, move |index| {
        let (ready, handle) = std::sync::mpsc::channel();
        let mut stopped = stopped.clone();
        std::thread::Builder::new()
            .name(format!("w-{index}"))
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("a worker's runtime");
                let _ = ready.send(runtime.handle().clone());
                runtime.block_on(async move {
                    let _ = stopped.changed().await;
                });
            })
            .ok()?;
        handle.recv().ok()
    })
    .expect("a first worker");
    (workers, stop)
}

fn channel_on(endpoint: &str, workers: &Workers, calls_per_connection: usize) -> GrpcChannel {
    let mut transport = TransportConfig::new(Uri::try_from(endpoint).expect("an endpoint"));
    transport.http2.simultaneous_calls_per_connection = Some(calls_per_connection);
    GrpcChannel::new(GrpcChannelConfig::new(transport), workers.clone()).expect("a channel")
}

/// The threads a call's response reached its sink on, and its end.
struct Threads {
    seen: Vec<String>,
    done: Option<oneshot::Sender<(Vec<String>, GrpcStatusCode)>>,
}

impl Threads {
    fn new() -> (Self, oneshot::Receiver<(Vec<String>, GrpcStatusCode)>) {
        let (done, heard) = oneshot::channel();
        let sink = Self {
            seen: Vec::new(),
            done: Some(done),
        };
        (sink, heard)
    }

    fn note(&mut self) {
        self.seen
            .push(std::thread::current().name().unwrap_or("?").to_owned());
    }
}

impl ResponseSink for Threads {
    async fn head(&mut self, _: ResponseHead) -> Result<(), GrpcStatus> {
        self.note();
        Ok(())
    }

    async fn message(&mut self, _: Bytes) -> Result<(), GrpcStatus> {
        self.note();
        Ok(())
    }

    fn flush(&mut self) {}

    async fn end(mut self, status: GrpcStatus, _: Option<ResponseHead>) {
        self.note();
        if let Some(done) = self.done.take() {
            let _ = done.send((std::mem::take(&mut self.seen), status.code));
        }
    }
}

type Heard = oneshot::Receiver<(Vec<String>, GrpcStatusCode)>;

/// Starts `count` echo calls on `channel`, each placed and spawned where it is placed, and answers
/// with what their sinks will hear.
///
/// Every call is placed before any request is sent and before this task yields: the test server
/// runs on this task's runtime, so no dial completes while the calls are placed, and the
/// placements alone decide where each call goes.
async fn placed_echoes(channel: &GrpcChannel, count: usize) -> Vec<Heard> {
    let mut sends = Vec::new();
    let mut heard = Vec::new();
    for _ in 0..count {
        let (send, _control, mut driver) = channel
            .prepare_call(CallStartOptions::new(ECHO))
            .expect("an open channel");
        let (sink, sunk) = Threads::new();
        let worker = driver.place();
        worker.spawn(driver.drive(sink));
        sends.push(send);
        heard.push(sunk);
    }
    for mut send in sends {
        send.send_message(Bytes::from_static(b"hello"))
            .await
            .expect("the call is open");
    }
    heard
}

/// Placed before any of them runs, four calls that each need a connection of their own open four
/// dials, given to the two workers in turn by their counts; each call is heard wholly on its
/// connection's worker.
#[tokio::test]
async fn each_call_runs_on_its_connections_worker_and_connections_spread_over_the_workers() {
    let server = TestServer::start().await;
    let (workers, _stop) = pool(2, WorkerAssignment::LeastConnections);
    let channel = channel_on(&server.endpoint, &workers, 1);

    let heard = placed_echoes(&channel, 4).await;
    let mut calls_per_worker = BTreeMap::new();
    for call in heard {
        let (threads, code) = call.await.expect("the call ends");
        assert_eq!(code, GrpcStatusCode::Ok);
        assert!(
            threads.iter().all(|thread| *thread == threads[0]),
            "a call moved between threads: {threads:?}"
        );
        *calls_per_worker.entry(threads[0].clone()).or_insert(0) += 1;
    }
    assert_eq!(
        calls_per_worker,
        BTreeMap::from([("w-0".to_owned(), 2), ("w-1".to_owned(), 2)])
    );
    assert_eq!(server.connections(), 4);
}

/// Calls placed together take their room as they are placed, so that they do not all read the
/// same room and open more connections than they need.
#[tokio::test]
async fn calls_placed_together_open_no_more_connections_than_they_need() {
    let server = TestServer::start().await;
    let (workers, _stop) = pool(4, WorkerAssignment::LeastConnections);
    let channel = channel_on(&server.endpoint, &workers, 4);

    let heard = placed_echoes(&channel, 16).await;
    let mut calls_per_worker = BTreeMap::new();
    for call in heard {
        let (threads, code) = call.await.expect("the call ends");
        assert_eq!(code, GrpcStatusCode::Ok);
        *calls_per_worker.entry(threads[0].clone()).or_insert(0) += 1;
    }
    assert_eq!(server.connections(), 4);
    assert_eq!(
        calls_per_worker.values().copied().collect::<Vec<_>>(),
        [4, 4, 4, 4]
    );
}

/// A call placed and gone before it needed its connection opens none, and gives back the count
/// its planned dial held against its worker: the next call is given that worker again.
#[tokio::test]
async fn a_call_that_ends_before_it_needs_its_connection_dials_nothing() {
    let server = TestServer::start().await;
    let (workers, _stop) = pool(2, WorkerAssignment::LeastConnections);
    let channel = channel_on(&server.endpoint, &workers, 1);

    let (_send, _control, mut driver) = channel
        .prepare_call(CallStartOptions::new(ECHO))
        .expect("an open channel");
    driver.place();
    drop(driver);

    let heard = placed_echoes(&channel, 1).await;
    for call in heard {
        let (threads, code) = call.await.expect("the call ends");
        assert_eq!(code, GrpcStatusCode::Ok);
        assert_eq!(
            threads[0], "w-0",
            "the dropped call's worker carries nothing"
        );
    }
    assert_eq!(server.connections(), 1, "the dropped call dialled nothing");
}

/// In turn, a pool gives each worker a connection whatever the others carry.
#[tokio::test]
async fn round_robin_gives_the_workers_connections_in_turn() {
    let server = TestServer::start().await;
    let (workers, _stop) = pool(3, WorkerAssignment::RoundRobin);
    let channel = channel_on(&server.endpoint, &workers, 1);

    let heard = placed_echoes(&channel, 3).await;
    let mut workers_seen = Vec::new();
    for call in heard {
        let (threads, _) = call.await.expect("the call ends");
        workers_seen.push(threads[0].clone());
    }
    workers_seen.sort();
    assert_eq!(workers_seen, ["w-0", "w-1", "w-2"]);
}
