//! What the estimate costs a call: the decisions on the hot path, alone, on one thread and on eight.
//!
//! A call that succeeds at its first attempt meets one decision to start (the cap, not capped, nobody
//! waiting) and records one count; one that fails meets the retry reading besides. Each is timed
//! here, with the threads of the multi-thread host contending on one channel's words and, for the
//! benchmark's last line, one thread of the C ABI's single-threaded runtime.
//!
//! Run it with `cargo bench -p armonik-transport --features test-hooks --bench admission`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use armonik_transport::grpc::AdaptiveConfig;
use armonik_transport::hooks::EstimateBench;

const OPERATIONS: u64 = 4_000_000;

/// The nanoseconds each operation takes, with `threads` threads each doing `OPERATIONS` of them on
/// the one estimate, while `writers` more threads record overload and accepts without end.
fn timed(
    threads: usize,
    writers: usize,
    operation: impl Fn(&EstimateBench) + Send + Sync + 'static,
) -> f64 {
    let estimate = Arc::new(EstimateBench::new(AdaptiveConfig::default()));
    let operation = Arc::new(operation);
    let stop = Arc::new(AtomicBool::new(false));
    let background: Vec<_> = (0..writers)
        .map(|which| {
            let estimate = Arc::clone(&estimate);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    if which % 2 == 0 {
                        estimate.record_accept();
                    } else {
                        estimate.record_overload();
                    }
                }
            })
        })
        .collect();

    let started = Instant::now();
    let workers: Vec<_> = (0..threads)
        .map(|_| {
            let estimate = Arc::clone(&estimate);
            let operation = Arc::clone(&operation);
            std::thread::spawn(move || {
                for _ in 0..OPERATIONS {
                    operation(&estimate);
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().expect("a worker");
    }
    let took = started.elapsed();
    stop.store(true, Ordering::Relaxed);
    for writer in background {
        writer.join().expect("a writer");
    }
    took.as_nanos() as f64 / OPERATIONS as f64
}

fn main() {
    println!("{:<48} {:>10}", "decision", "ns each");

    for (threads, writers) in [(1usize, 0usize), (8, 0)] {
        let record = timed(threads, writers, |estimate| estimate.record_accept());
        let retry = timed(threads, writers, |estimate| {
            std::hint::black_box(estimate.retries_open());
        });
        let both = timed(threads, writers, |estimate| {
            estimate.record_accept();
            std::hint::black_box(estimate.retries_open());
        });
        println!(
            "{:<48} {:>10.1}",
            format!("record, {threads} thread(s)"),
            record
        );
        println!(
            "{:<48} {:>10.1}",
            format!("retry reading, {threads} thread(s)"),
            retry
        );
        println!(
            "{:<48} {:>10.1}",
            format!("record and read, {threads} thread(s)"),
            both
        );
    }

    // Eight threads reading the retry gate while two more keep recording: the cache line the
    // records write is the one every read loads.
    let contended = timed(8, 2, |estimate| {
        std::hint::black_box(estimate.retries_open());
    });
    println!(
        "{:<48} {:>10.1}",
        "retry reading, 8 threads, 2 recording", contended
    );

    // The decision a first attempt meets with nothing capped, as the C ABI's one thread meets it.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("a runtime");
    let estimate = EstimateBench::new(AdaptiveConfig::default());
    let started = Instant::now();
    runtime.block_on(async {
        for _ in 0..OPERATIONS {
            estimate.first_attempt().await;
        }
    });
    println!(
        "{:<48} {:>10.1}",
        "first attempt, uncapped, current-thread runtime",
        started.elapsed().as_nanos() as f64 / OPERATIONS as f64
    );
}
