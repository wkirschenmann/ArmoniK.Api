//! Point 3: the load's events are kept and delivered at registration; the callback's lifetime.

use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use observability_spike::obs::Refusal;
use observability_spike::record::LogRecord;
use observability_spike::runtime::{Front, ObsRuntime};
use observability_spike::testkit::{collect, Collector};

fn load_events() {
    tracing::debug!(target: "armonik_transport_ffi::config", "load debug");
    tracing::info!(target: "armonik_transport_ffi::config", "load info");
    tracing::warn!(target: "armonik_transport_ffi::config", path = "Transport.Proxx", "load warn");
    tracing::info!(target: "h2::codec", "load h2 info");
}

#[test]
fn the_loads_events_are_kept_and_selected_by_the_filter_at_registration() {
    let runtime = ObsRuntime::new(Front::Layered);
    runtime.obs.begin_load();
    {
        let _inside = runtime.scope();
        load_events();
    }
    runtime.obs.end_load();
    assert_eq!(runtime.obs.kept_len(), 4, "everything up to debug is kept");

    // Later events, with no callback, are neither kept nor delivered.
    {
        let _inside = runtime.scope();
        tracing::warn!(target: "armonik_transport_ffi::config", "after the load");
    }
    assert_eq!(runtime.obs.kept_len(), 4);

    let log = Box::new(Collector::default());
    runtime.obs.set_log_callback(collect, log.ctx(), "").unwrap();
    // The default filter: the engine at info, h2 at warn.
    assert_eq!(log.messages(), ["load info", "load warn"]);
    assert_eq!(runtime.obs.kept_len(), 0, "delivered once");
    let records = log.take();
    assert_eq!(records[1].fields, [("path".to_owned(), "Transport.Proxx".to_owned())]);
}

#[test]
fn a_filter_given_at_registration_selects_the_kept_events() {
    let runtime = ObsRuntime::new(Front::Layered);
    runtime.obs.begin_load();
    {
        let _inside = runtime.scope();
        load_events();
    }
    runtime.obs.end_load();
    let log = Box::new(Collector::default());
    runtime
        .obs
        .set_log_callback(collect, log.ctx(), "debug,h2=trace")
        .unwrap();
    assert_eq!(log.messages(), ["load debug", "load info", "load warn", "load h2 info"]);
}

#[test]
fn a_runtime_that_never_registers_holds_only_the_loads_events() {
    let runtime = ObsRuntime::new(Front::Layered);
    runtime.obs.begin_load();
    {
        let _inside = runtime.scope();
        load_events();
    }
    runtime.obs.end_load();
    let channel = runtime.start_channel_thread();
    for _ in 0..1000 {
        runtime
            .tokio
            .block_on(channel.handle.spawn(async {
                tracing::warn!(target: "armonik_transport::grpc", "long after");
            }))
            .unwrap();
    }
    assert_eq!(runtime.obs.kept_len(), 4);
}

/// Events logged while the kept ones are delivered queue behind them: none lost after the first
/// delivered, none twice, in order.
#[test]
fn events_during_registration_follow_the_kept_ones_in_order() {
    for _ in 0..50 {
        let runtime = ObsRuntime::new(Front::Layered);
        runtime.obs.begin_load();
        {
            let _inside = runtime.scope();
            for i in 0..200 {
                tracing::info!(target: "armonik_transport_ffi::config", n = i as u64, "kept");
            }
        }
        runtime.obs.end_load();
        let channel = runtime.start_channel_thread();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let counter = Arc::new(AtomicUsize::new(1000));
        let (s, c) = (stop.clone(), counter.clone());
        let producer = channel.handle.spawn(async move {
            while !s.load(Ordering::Relaxed) {
                let n = c.fetch_add(1, Ordering::Relaxed);
                tracing::info!(target: "armonik_transport::grpc", n = n as u64, "live");
                tokio::task::yield_now().await;
            }
        });
        let log = Box::new(Collector::default());
        runtime.obs.set_log_callback(collect, log.ctx(), "").unwrap();
        std::thread::sleep(Duration::from_millis(5));
        stop.store(true, Ordering::Relaxed);
        runtime.tokio.block_on(producer).unwrap();

        let numbers: Vec<u64> = log
            .take()
            .iter()
            .map(|r| r.fields.iter().find(|f| f.0 == "n").unwrap().1.parse().unwrap())
            .collect();
        assert!(numbers.len() >= 200);
        assert_eq!(&numbers[..200], &(0..200).collect::<Vec<_>>()[..], "kept first, in order");
        let live = &numbers[200..];
        assert!(live.windows(2).all(|w| w[1] == w[0] + 1), "no gap, no repeat: {live:?}");
    }
}

static REENTRANT_CALLS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn logs_again(_: *mut c_void, _: *const LogRecord) {
    REENTRANT_CALLS.fetch_add(1, Ordering::Relaxed);
    // As an ak_* call made from inside the callback would.
    tracing::info!(target: "armonik_transport::grpc", "logged from inside the callback");
}

#[test]
fn an_event_logged_from_inside_the_callback_is_dropped() {
    let runtime = ObsRuntime::new(Front::Layered);
    runtime
        .obs
        .set_log_callback(logs_again, std::ptr::null_mut(), "")
        .unwrap();
    let _inside = runtime.scope();
    tracing::info!(target: "armonik_transport::grpc", "outer");
    assert_eq!(REENTRANT_CALLS.load(Ordering::Relaxed), 1);
}

struct Slow {
    entered: AtomicUsize,
    finished: Mutex<Option<Instant>>,
}

unsafe extern "C" fn slow(ctx: *mut c_void, _: *const LogRecord) {
    let slow = unsafe { &*(ctx as *const Slow) };
    slow.entered.fetch_add(1, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(150));
    *slow.finished.lock().unwrap() = Some(Instant::now());
}

/// After `clear_log_callback` returns, no thread is inside the callback and none will enter it.
#[test]
fn clearing_the_callback_waits_for_the_invocation_under_way() {
    let runtime = Arc::new(ObsRuntime::new(Front::Layered));
    let state = Arc::new(Slow {
        entered: AtomicUsize::new(0),
        finished: Mutex::new(None),
    });
    runtime
        .obs
        .set_log_callback(slow, Arc::as_ptr(&state) as *mut c_void, "")
        .unwrap();
    let emitter = {
        let runtime = runtime.clone();
        std::thread::spawn(move || {
            let _inside = runtime.scope();
            tracing::info!(target: "armonik_transport::grpc", "slow one");
        })
    };
    while state.entered.load(Ordering::SeqCst) == 0 {
        std::thread::yield_now();
    }
    let started = Instant::now();
    runtime.obs.clear_log_callback().unwrap();
    let waited = started.elapsed();
    let finished = state.finished.lock().unwrap().expect("the callback finished first");
    assert!(finished <= Instant::now());
    println!("clear_log_callback waited {waited:?}");
    assert!(waited >= Duration::from_millis(50));
    emitter.join().unwrap();

    // Nothing reaches it any more.
    {
        let _inside = runtime.scope();
        tracing::info!(target: "armonik_transport::grpc", "after the clear");
    }
    assert_eq!(state.entered.load(Ordering::SeqCst), 1);
}

unsafe extern "C" fn clears_itself(ctx: *mut c_void, _: *const LogRecord) {
    let runtime = unsafe { &*(ctx as *const ObsRuntime) };
    let answer = runtime.obs.clear_log_callback();
    assert_eq!(answer, Err(Refusal::FromInsideCallback));
}

#[test]
fn clearing_from_inside_the_callback_is_refused() {
    let runtime = ObsRuntime::new(Front::Layered);
    runtime
        .obs
        .set_log_callback(clears_itself, &runtime as *const ObsRuntime as *mut c_void, "")
        .unwrap();
    let _inside = runtime.scope();
    tracing::info!(target: "armonik_transport::grpc", "trigger");
}
