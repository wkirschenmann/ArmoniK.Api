//! Point 2: changing the filter while library threads log.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use observability_spike::record::{LogRecord, AK_LOG_DEBUG, AK_LOG_INFO};
use observability_spike::runtime::{Front, ObsRuntime};

static INFOS: AtomicU64 = AtomicU64::new(0);
static DEBUGS: AtomicU64 = AtomicU64::new(0);

unsafe extern "C" fn by_level(_: *mut c_void, record: *const LogRecord) {
    match unsafe { (*record).level } {
        AK_LOG_INFO => INFOS.fetch_add(1, Ordering::Relaxed),
        AK_LOG_DEBUG => DEBUGS.fetch_add(1, Ordering::Relaxed),
        _ => 0,
    };
}

/// No hang, no panic, and every event the filter allowed all along crosses.
#[test]
fn changing_the_filter_under_load_neither_hangs_nor_loses_the_stream() {
    let runtime = ObsRuntime::new(Front::Layered);
    runtime
        .obs
        .set_log_callback(by_level, std::ptr::null_mut(), "info")
        .unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let emitted = Arc::new(AtomicU64::new(0));
    let mut tasks = Vec::new();
    for _ in 0..4 {
        let channel = runtime.start_channel_thread();
        let (stop, emitted) = (stop.clone(), emitted.clone());
        let task = channel.handle.spawn(async move {
            while !stop.load(Ordering::Relaxed) {
                tracing::info!(target: "armonik_transport::load", "info under load");
                tracing::debug!(target: "armonik_transport::load", "debug under load");
                emitted.fetch_add(1, Ordering::Relaxed);
                tokio::task::yield_now().await;
            }
        });
        tasks.push((channel, task));
    }
    let started = Instant::now();
    let mut worst = Duration::ZERO;
    for round in 0..300 {
        let begun = Instant::now();
        runtime
            .obs
            .set_filter(if round % 2 == 0 { "debug" } else { "info" })
            .unwrap();
        worst = worst.max(begun.elapsed());
        // Let the emitting threads run under this filter for a moment.
        std::thread::sleep(Duration::from_micros(300));
    }
    stop.store(true, Ordering::Relaxed);
    for (_channel, task) in tasks {
        runtime.tokio.block_on(task).unwrap();
    }
    let (infos, debugs) = (INFOS.load(Ordering::Relaxed), DEBUGS.load(Ordering::Relaxed));
    println!(
        "300 set_filter calls under 4 emitting threads took {:?}, worst one {worst:?}; {} loops emitted, {infos} info and {debugs} debug delivered",
        started.elapsed(),
        emitted.load(Ordering::Relaxed)
    );
    assert_eq!(infos, emitted.load(Ordering::Relaxed), "every info crossed, once");
    assert!(debugs > 0, "debug crossed while the filter allowed it");
}
