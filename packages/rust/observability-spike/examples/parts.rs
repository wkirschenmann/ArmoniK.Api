//! Point 2: what the pieces of a `sometimes` decision cost, one at a time.

use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

use arc_swap::ArcSwap;
use observability_spike::obs::{parse_filter, DEFAULT_FILTER};
use tracing::dispatcher::{self, Dispatch};
use tracing::{Level, Metadata};
use tracing_core::LevelFilter;
use tracing_subscriber::filter::Targets;

struct Tiny;
impl tracing::Subscriber for Tiny {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        false
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, _: &tracing::Event<'_>) {}
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

thread_local! {
    static TL: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn time(label: &str, n: u64, mut f: impl FnMut(u64)) {
    let mut samples = Vec::new();
    for _ in 0..7 {
        let start = Instant::now();
        for i in 0..n {
            f(black_box(i));
        }
        samples.push(start.elapsed().as_secs_f64() * 1e9 / n as f64);
    }
    samples.sort_by(f64::total_cmp);
    println!("{label:60} min {:6.2}  median {:6.2}  max {:6.2} ns", samples[0], samples[3], samples[6]);
}

static CALLSITE_PROBE: std::sync::OnceLock<&'static Metadata<'static>> = std::sync::OnceLock::new();

fn main() {
    let n = 10_000_000;
    let targets = parse_filter(DEFAULT_FILTER).unwrap();
    let swap = ArcSwap::from_pointee(targets.clone());
    let plain: Arc<Targets> = Arc::new(targets.clone());

    // A metadata to ask about.
    let meta: &'static Metadata<'static> = {
        struct Probe;
        impl tracing::Subscriber for Probe {
            fn enabled(&self, m: &Metadata<'_>) -> bool {
                let _ = CALLSITE_PROBE.set(unsafe { std::mem::transmute::<&Metadata<'_>, &'static Metadata<'static>>(m) });
                false
            }
            fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
                tracing::span::Id::from_u64(1)
            }
            fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
            fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
            fn event(&self, _: &tracing::Event<'_>) {}
            fn enter(&self, _: &tracing::span::Id) {}
            fn exit(&self, _: &tracing::span::Id) {}
        }
        dispatcher::with_default(&Dispatch::new(Probe), || {
            tracing::debug!(target: "armonik_transport::hot", "probe")
        });
        CALLSITE_PROBE.get().copied().expect("probed")
    };

    time("thread_local Cell get", n, |i| {
        TL.with(|c| c.set(c.get() + i));
    });
    time("ArcSwap::load + deref", n, |_| {
        black_box(swap.load().default_level());
    });
    time("Arc clone + drop", n, |_| {
        black_box(plain.clone());
    });
    time("Targets::would_enable (3 directives, matches h2 fallback)", n, |_| {
        black_box(plain.would_enable(black_box("armonik_transport::hot"), &Level::DEBUG));
    });
    time("Targets::would_enable (target of an h2 module)", n, |_| {
        black_box(plain.would_enable(black_box("h2::proto::connection"), &Level::DEBUG));
    });

    let fast = observability_spike::fast_filter::FastFilter::from_targets(&targets);
    let cell = observability_spike::fast_filter::FilterCell::new(
        observability_spike::fast_filter::FastFilter::from_targets(&targets),
    );
    let rwlock = std::sync::RwLock::new(plain.clone());
    time("FastFilter::would_enable (same 3 directives)", n, |_| {
        black_box(fast.would_enable(black_box("armonik_transport::hot"), &Level::DEBUG));
    });
    time("FilterCell::get + FastFilter::would_enable", n, |_| {
        black_box(cell.get().would_enable(black_box("armonik_transport::hot"), &Level::DEBUG));
    });
    time("RwLock<Arc>::read + clone-free Targets::would_enable", n, |_| {
        black_box(rwlock.read().unwrap().would_enable(black_box("armonik_transport::hot"), &Level::DEBUG));
    });
    let atomic = std::sync::atomic::AtomicU64::new(0);
    time("AtomicU64 fetch_add relaxed (1 thread)", n, |_| {
        atomic.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    });
    time("AtomicU64 load+store relaxed (1 writer)", n, |_| {
        atomic.store(atomic.load(std::sync::atomic::Ordering::Relaxed) + 1, std::sync::atomic::Ordering::Relaxed);
    });

    let _guard = dispatcher::set_default(&Dispatch::new(Tiny));
    time("dispatcher::get_default + Tiny::enabled (scoped, 1 thread)", n, |_| {
        black_box(dispatcher::get_default(|d| d.enabled(meta)));
    });
    time("dispatcher::get_default + RtSub-like (ArcSwap + Targets)", n, |_| {
        black_box(dispatcher::get_default(|_| swap.load().would_enable(meta.target(), meta.level())));
    });

    // The guard FFI calls would take around their body.
    let dispatch = Dispatch::new(Tiny);
    time("set_default guard create + drop (an FFI scope)", n, |_| {
        black_box(dispatcher::set_default(&dispatch));
    });
    let _ = LevelFilter::OFF;
}
