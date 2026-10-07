//! Point 5: what a counter increment costs on a hot path, and what a read of the runtime's
//! statistics costs.
//!
//! stats_cost <threads>

use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::time::Instant;

use observability_spike::stats::{ChannelStats, Counter, RuntimeStats, Shared, StatsV1};

const N: u64 = 20_000_000;

fn median(mut samples: Vec<f64>) -> (f64, f64, f64) {
    samples.sort_by(f64::total_cmp);
    (samples[0], samples[samples.len() / 2], samples[samples.len() - 1])
}

fn per_thread<T: Send + Sync + 'static>(
    threads: usize,
    make: impl Fn(usize) -> Arc<T>,
    bump: fn(&T),
) -> (f64, f64, f64) {
    let mut runs = Vec::new();
    for _ in 0..5 {
        let barrier = Arc::new(Barrier::new(threads));
        let handles: Vec<_> = (0..threads)
            .map(|i| {
                let target = make(i);
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let start = Instant::now();
                    for _ in 0..N {
                        bump(black_box(&target));
                    }
                    start.elapsed().as_secs_f64() * 1e9 / N as f64
                })
            })
            .collect();
        let worst = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .fold(0.0, f64::max);
        runs.push(worst);
    }
    median(runs)
}

fn main() {
    let threads: usize = std::env::args().nth(1).map_or(4, |v| v.parse().unwrap());
    let show = |label: &str, (min, med, max): (f64, f64, f64)| {
        println!("{label:62} min {min:6.2}  median {med:6.2}  max {max:6.2} ns/op (slowest thread)")
    };

    show(
        "1 thread:  single-writer Counter (load+store)",
        per_thread(1, |_| Arc::new(ChannelStats::default()), |s| s.dials.add(1)),
    );
    show(
        "1 thread:  Shared (fetch_add)",
        per_thread(1, |_| Arc::new(ChannelStats::default()), |s| s.calls_started.add(1)),
    );
    let one = Arc::new(ChannelStats::default());
    show(
        &format!("{threads} threads: one Shared counter, all threads (contended)"),
        per_thread(threads, |_| one.clone(), |s| s.calls_started.add(1)),
    );
    show(
        &format!("{threads} threads: one Shared per thread, padded (own channel)"),
        per_thread(threads, |_| Arc::new(ChannelStats::default()), |s| s.calls_started.add(1)),
    );
    show(
        &format!("{threads} threads: one Counter per thread, padded (own channel)"),
        per_thread(threads, |_| Arc::new(ChannelStats::default()), |s| s.dials.add(1)),
    );
    let unpadded: Arc<Vec<AtomicU64>> = Arc::new((0..threads).map(|_| AtomicU64::new(0)).collect());
    let _ = &unpadded;
    show(
        &format!("{threads} threads: adjacent AtomicU64s, unpadded (false sharing)"),
        per_thread(
            threads,
            |i| {
                let _ = i;
                unpadded.clone()
            },
            |v| {
                let index = std::thread::current().name().map_or(0, |_| 0);
                let _ = index;
                // Each thread hits slot (thread id mod len): adjacent slots share a line.
                let slot = (thread_slot()) % v.len();
                v[slot].fetch_add(1, Ordering::Relaxed);
            },
        ),
    );

    // Reads.
    println!();
    for channels in [1usize, 10, 100, 1000] {
        let runtime = Arc::new(RuntimeStats::default());
        let handles: Vec<_> = (0..channels).map(|_| runtime.open_channel()).collect();
        for handle in &handles {
            handle.stats.calls_started.add(5);
            handle.stats.dials.add(1);
        }
        // With writers running, as in production.
        let stop = Arc::new(AtomicBool::new(false));
        let writers: Vec<_> = (0..threads.min(handles.len()))
            .map(|i| {
                let stats = handles[i].stats.clone();
                let stop = stop.clone();
                std::thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        stats.dials.add(1);
                        stats.calls_started.add(1);
                        std::hint::spin_loop();
                    }
                })
            })
            .collect();
        let mut out = StatsV1 {
            struct_size: std::mem::size_of::<StatsV1>() as u32,
            ..Default::default()
        };
        let mut samples = Vec::new();
        for _ in 0..200 {
            let start = Instant::now();
            runtime.read(&mut out);
            samples.push(start.elapsed().as_secs_f64() * 1e6);
        }
        stop.store(true, Ordering::Relaxed);
        for writer in writers {
            writer.join().unwrap();
        }
        let (min, med, max) = median(samples);
        println!(
            "read of {channels:5} channels: min {min:8.2}  median {med:8.2}  max {max:8.2} us   (channels_open={})",
            out.channels_open
        );
        drop(handles);
    }
    let _ = (black_box(Counter::default()), black_box(Shared::default()));
}

fn thread_slot() -> usize {
    thread_local! {
        static SLOT: usize = {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            NEXT.fetch_add(1, Ordering::Relaxed) as usize
        };
    }
    SLOT.with(|slot| *slot)
}
