//! What the counting costs a call: unary calls over loopback to a server of another runtime, from
//! a current-thread runtime, which is a channel's thread in the C ABI, and from tokio's multi-thread
//! runtime, which a Rust host has.
//!
//! Run it twice and compare, once with the `metrics` feature and once without:
//! `cargo bench -p armonik-transport --bench metrics` and
//! `cargo bench -p armonik-transport --bench metrics --features metrics`.
//! The arguments, after `--`, are the calls of a repetition, how many of them run at once, and the
//! repetitions.
//! With the `test-hooks` feature too, each counting point is timed alone, on one thread and on eight.

#[allow(dead_code)]
#[path = "../tests/common/mod.rs"]
mod common;

use std::time::Instant;

use armonik_transport::grpc::{CallStartOptions, GrpcChannel};
use bytes::Bytes;
use common::echo::{channel, unary, TestServer, ECHO};

/// Calls in flight at once, `concurrency` tasks of the runtime each making an equal share; how many
/// were made.
async fn calls(channel: &GrpcChannel, total: usize, concurrency: usize) -> usize {
    let share = total / concurrency.max(1);
    let tasks: Vec<_> = (0..concurrency)
        .map(|_| {
            let channel = channel.clone();
            tokio::spawn(async move {
                for _ in 0..share {
                    let (_, _, status) = unary(
                        &channel,
                        CallStartOptions::new(ECHO),
                        Bytes::from_static(b"hello"),
                    )
                    .await;
                    assert!(status.code == armonik_transport::grpc::GrpcStatusCode::Ok);
                }
            })
        })
        .collect();
    for task in tasks {
        task.await.expect("a task finished");
    }
    share * concurrency.max(1)
}

/// The microseconds each call takes, per repetition, on `runtime`.
fn run(
    runtime: &tokio::runtime::Runtime,
    endpoint: &str,
    total: usize,
    concurrency: usize,
    repeats: usize,
) -> Vec<f64> {
    runtime.block_on(async {
        let channel = channel(endpoint);
        // Warm: the connection, the allocator, the caches.
        calls(&channel, total / 4, concurrency).await;
        let mut taken = Vec::new();
        for _ in 0..repeats {
            let started = Instant::now();
            let made = calls(&channel, total, concurrency).await;
            taken.push(started.elapsed().as_secs_f64() * 1e6 / made as f64);
        }
        taken
    })
}

fn report(what: &str, mut taken: Vec<f64>) {
    taken.sort_by(f64::total_cmp);
    println!(
        "{what:<34} min {:>7.2} us/call   median {:>7.2} us/call",
        taken[0],
        taken[taken.len() / 2]
    );
}

/// The nanoseconds each operation takes, `threads` threads each doing `OPERATIONS` of them with
/// counters of their own and a registry they share, timed from when all of them are ready.
#[cfg(feature = "test-hooks")]
fn point(
    threads: usize,
    operation: impl Fn(&mut armonik_transport::hooks::CountingBench) + Send + Sync + 'static,
) -> f64 {
    use armonik_transport::hooks::CountingBench;
    use armonik_transport::metrics::Metrics;

    const OPERATIONS: u64 = 2_000_000;
    let metrics = Metrics::new();
    let operation = std::sync::Arc::new(operation);
    let mut best = f64::MAX;
    for _ in 0..5 {
        let ready = std::sync::Arc::new(std::sync::Barrier::new(threads));
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                let metrics = metrics.clone();
                let operation = operation.clone();
                let ready = ready.clone();
                std::thread::spawn(move || {
                    let mut bench = CountingBench::new(metrics);
                    ready.wait();
                    let started = Instant::now();
                    for _ in 0..OPERATIONS {
                        operation(&mut bench);
                    }
                    started.elapsed().as_secs_f64() * 1e9 / OPERATIONS as f64
                })
            })
            .collect();
        let slowest = workers
            .into_iter()
            .map(|worker| worker.join().expect("a worker finished"))
            .fold(0.0, f64::max);
        best = best.min(slowest);
    }
    best
}

#[cfg(feature = "test-hooks")]
fn points() {
    println!("each counting point alone, ns per operation (best of five):");
    for threads in [1, 8] {
        println!(
            "  {threads} thread(s): a message sent and received {:>7.1}   a 16 KiB read, counted and its frame read {:>7.1}   a call in the registry {:>7.1}   a retry {:>7.1}",
            point(threads, |bench| bench.messages()),
            point(threads, |bench| bench.read()),
            point(threads, |bench| bench.call_lifecycle()),
            point(threads, |bench| bench.retry()),
        );
    }
}

fn main() {
    let args: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|arg| arg.parse().ok())
        .collect();
    let total = args.first().copied().unwrap_or(20_000);
    let concurrency = args.get(1).copied().unwrap_or(32);
    let repeats = args.get(2).copied().unwrap_or(15);

    // The server on a runtime of its own, apart from the client's, which it still shares the CPU with.
    let server_runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("the server's runtime");
    let server = server_runtime.block_on(TestServer::start());

    println!(
        "metrics feature {}: {total} calls a repetition, {concurrency} at once, {repeats} repetitions",
        if cfg!(feature = "metrics") { "on" } else { "off" }
    );

    #[cfg(feature = "test-hooks")]
    points();

    let one_thread = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a current-thread runtime");
    report(
        "current-thread (a C ABI channel)",
        run(&one_thread, &server.endpoint, total, concurrency, repeats),
    );

    let many_threads = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("a multi-thread runtime");
    report(
        "multi-thread, four workers",
        run(&many_threads, &server.endpoint, total, concurrency, repeats),
    );
}
