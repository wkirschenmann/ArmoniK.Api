//! Points 3 and 4: allocations per delivered event, per kept event and per span.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

use observability_spike::obs::RtObs;
use observability_spike::runtime::{Front, ObsRuntime};
use observability_spike::testkit::{count, Counter};
use observability_spike::trace::TraceRecord;

struct Counting;
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static A: Counting = Counting;

unsafe extern "C" fn ignore_span(_: *mut std::ffi::c_void, _: *const TraceRecord) {}

fn measure(label: &str, n: u64, mut f: impl FnMut(u64)) {
    // Warm up: the thread-local buffers reach their size.
    for i in 0..1000 {
        f(i);
    }
    let (a0, b0) = (ALLOCS.load(Ordering::Relaxed), BYTES.load(Ordering::Relaxed));
    for i in 0..n {
        f(i);
    }
    let (a1, b1) = (ALLOCS.load(Ordering::Relaxed), BYTES.load(Ordering::Relaxed));
    println!(
        "{label:60} {:6.2} allocations, {:8.1} bytes per iteration",
        (a1 - a0) as f64 / n as f64,
        (b1 - b0) as f64 / n as f64
    );
}

fn main() {
    let counter: &'static Counter = Box::leak(Box::new(Counter::default()));
    let runtime = ObsRuntime::new(Front::Layered);
    runtime.obs.set_log_callback(count, counter.ctx(), "info").unwrap();
    let _inside = runtime.scope();

    measure("info event, 2 fields + message, delivered live", 100_000, |i| {
        tracing::info!(target: "armonik_transport::grpc::channel", endpoint = "http://10.0.0.1:5001", attempt = i, "dial started");
    });
    measure("info event, a Display field (%) and a Debug field (?)", 100_000, |i| {
        let error = std::io::Error::other("connection refused");
        let peer = std::net::SocketAddr::from(([10, 0, 0, 1], 5001));
        tracing::info!(target: "armonik_transport::grpc::channel", %peer, ?error, attempt = i, "dial failed");
    });
    measure("debug event below the filter", 100_000, |i| {
        tracing::debug!(target: "armonik_transport::grpc::channel", attempt = i, "x");
    });

    // The kept events: copied to owned strings.
    let kept = ObsRuntime::new(Front::Layered);
    kept.obs.begin_load();
    {
        let _scope = kept.scope();
        measure("kept event while loading (OwnedRecord)", 10_000, |i| {
            tracing::info!(target: "armonik_transport_ffi::config", path = "Transport.Proxx", attempt = i, "unknown configuration key ignored");
        });
    }

    // Spans.
    let traced = ObsRuntime::new(Front::Layered);
    traced
        .obs
        .trace
        .register(ignore_span, std::ptr::null_mut(), "armonik_transport::trace=info")
        .unwrap();
    let _traced = traced.scope();
    measure("span: built, entered, closed, one attribute", 100_000, |i| {
        let span = tracing::info_span!(target: "armonik_transport::trace::attempt", "attempt", attempt = i);
        let _e = span.enter();
    });
    measure("span with a child, four attributes in all", 100_000, |i| {
        let call = tracing::info_span!(target: "armonik_transport::trace::attempt", "call", method = "/x/Y", attempt = i, endpoint = "http://127.0.0.1:5001");
        let child = tracing::info_span!(target: "armonik_transport::trace::attempt", parent: &call, "dial", attempt = i);
        let _e = child.enter();
    });
    let _ = RtObs::new;
}
