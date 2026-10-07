//! Point 1 and 2: with ONE dispatcher in the process, tracing registers a callsite against the
//! dispatcher of the thread that hits it first. Run alone in its process: the dispatcher list is
//! process-wide.

use std::sync::atomic::Ordering;

use observability_spike::runtime::{Front, ObsRuntime, USE_SENTINEL};
use observability_spike::testkit::{collect, Collector};

fn first_hit() {
    tracing::info!(target: "armonik_transport::just_one", "first hit");
}

fn second_hit() {
    tracing::info!(target: "armonik_transport::just_one", "second hit");
}

fn run() -> Vec<String> {
    let runtime = ObsRuntime::new(Front::Bare);
    let collector = Box::new(Collector::default());
    runtime
        .obs
        .set_log_callback(collect, collector.ctx(), "")
        .unwrap();
    let channel = runtime.start_channel_thread();

    // A thread that is under no runtime reaches the callsite first.
    std::thread::spawn(first_hit).join().unwrap();
    // Then the runtime's own thread reaches the same one.
    runtime
        .tokio
        .block_on(channel.handle.spawn(async { first_hit() }))
        .unwrap();
    // A callsite the runtime's thread reaches first is fine.
    runtime
        .tokio
        .block_on(channel.handle.spawn(async { second_hit() }))
        .unwrap();
    std::thread::spawn(second_hit).join().unwrap();
    collector.messages()
}

/// One test, so that the process has the one dispatcher the trap needs, then the sentinel's.
#[test]
fn a_callsite_first_reached_outside_the_only_dispatcher_is_lost_to_it() {
    USE_SENTINEL.store(false, Ordering::Relaxed);
    let without = run();
    println!("without the sentinel: {without:?}");
    // The runtime's thread reached `first_hit` after a thread under no runtime had cached `never`.
    assert_eq!(without, ["second hit"]);

    USE_SENTINEL.store(true, Ordering::Relaxed);
    // New callsites, as the cache of the first two is what is under test: the same functions
    // would still hold their cached `never`, so the sentinel's effect is shown on fresh ones.
    fn third_hit() {
        tracing::info!(target: "armonik_transport::just_one", "third hit");
    }
    let runtime = ObsRuntime::new(Front::Bare);
    let collector = Box::new(Collector::default());
    runtime
        .obs
        .set_log_callback(collect, collector.ctx(), "")
        .unwrap();
    let channel = runtime.start_channel_thread();
    std::thread::spawn(third_hit).join().unwrap();
    runtime
        .tokio
        .block_on(channel.handle.spawn(async { third_hit() }))
        .unwrap();
    println!("with the sentinel: {:?}", collector.messages());
    assert_eq!(collector.messages(), ["third hit"]);
}
