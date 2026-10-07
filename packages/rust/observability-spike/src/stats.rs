//! Point 5: counters and gauges per channel, summed per runtime, read into a versioned structure.
//!
//! A channel's thread runs the channel's connection and every call on it, so most counters have one
//! writer; a counter that the host's thread also moves (a call started from `ak_call_start`) has
//! two. `Counter` is the cheap form for one writer, `Shared` the atomic read-modify-write form.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

/// A counter written from one thread and read from any: a plain load and store, no `lock` prefix.
#[derive(Default)]
#[repr(transparent)]
pub struct Counter(AtomicU64);

impl Counter {
    /// The caller is the one writer.
    #[inline]
    pub fn add(&self, by: u64) {
        self.0.store(self.0.load(Ordering::Relaxed).wrapping_add(by), Ordering::Relaxed);
    }

    #[inline]
    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// A counter any thread may move.
#[derive(Default)]
#[repr(transparent)]
pub struct Shared(AtomicU64);

impl Shared {
    #[inline]
    pub fn add(&self, by: u64) {
        self.0.fetch_add(by, Ordering::Relaxed);
    }

    #[inline]
    pub fn sub(&self, by: u64) {
        self.0.fetch_sub(by, Ordering::Relaxed);
    }

    #[inline]
    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// One channel's numbers. Padded to its own cache lines: channels run on threads of their own, and
/// a runtime's channels sit next to each other in memory otherwise.
#[derive(Default)]
#[repr(align(128))]
pub struct ChannelStats {
    // Written on the channel's thread.
    pub dials: Counter,
    pub dial_failures: Counter,
    pub retries: Counter,
    pub stream_resets: Counter,
    pub goaways: Counter,
    pub calls_completed: Counter,
    pub calls_failed: Counter,
    // Written by the host's thread and the channel's.
    pub calls_started: Shared,
    pub calls_in_flight: Shared,
}

/// What `ak_runtime_stats` fills. `struct_size` is the caller's, and what the library wrote on
/// return: a host built against an older header reads the prefix it knows.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StatsV1 {
    pub struct_size: u32,
    pub version: u32,
    pub channels_open: u64,
    pub calls_started: u64,
    pub calls_in_flight: u64,
    pub calls_completed: u64,
    pub calls_failed: u64,
    pub dials: u64,
    pub dial_failures: u64,
    pub retries: u64,
    pub stream_resets: u64,
    pub goaways: u64,
}

#[derive(Default)]
struct Retired {
    calls_started: u64,
    calls_completed: u64,
    calls_failed: u64,
    dials: u64,
    dial_failures: u64,
    retries: u64,
    stream_resets: u64,
    goaways: u64,
}

/// The runtime's view: the channels it holds, and what the closed ones counted before they went.
#[derive(Default)]
pub struct RuntimeStats {
    inner: Mutex<(Vec<Weak<ChannelStats>>, Retired)>,
}

/// Held by a channel; when it drops, its counters join the runtime's retired totals, so a
/// monotonic counter never goes backwards when a channel closes.
pub struct ChannelStatsHandle {
    pub stats: Arc<ChannelStats>,
    runtime: Arc<RuntimeStats>,
}

impl RuntimeStats {
    pub fn open_channel(self: &Arc<Self>) -> ChannelStatsHandle {
        let stats = Arc::new(ChannelStats::default());
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .0
            .push(Arc::downgrade(&stats));
        ChannelStatsHandle {
            stats,
            runtime: self.clone(),
        }
    }

    /// Fills as much of `out` as its `struct_size` says it holds, and says how much that was.
    pub fn read(&self, out: &mut StatsV1) {
        let guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let mut sum = StatsV1 {
            struct_size: std::mem::size_of::<StatsV1>() as u32,
            version: 1,
            calls_started: guard.1.calls_started,
            calls_completed: guard.1.calls_completed,
            calls_failed: guard.1.calls_failed,
            dials: guard.1.dials,
            dial_failures: guard.1.dial_failures,
            retries: guard.1.retries,
            stream_resets: guard.1.stream_resets,
            goaways: guard.1.goaways,
            ..StatsV1::default()
        };
        for channel in guard.0.iter().filter_map(Weak::upgrade) {
            sum.channels_open += 1;
            sum.calls_started += channel.calls_started.get();
            sum.calls_in_flight += channel.calls_in_flight.get();
            sum.calls_completed += channel.calls_completed.get();
            sum.calls_failed += channel.calls_failed.get();
            sum.dials += channel.dials.get();
            sum.dial_failures += channel.dial_failures.get();
            sum.retries += channel.retries.get();
            sum.stream_resets += channel.stream_resets.get();
            sum.goaways += channel.goaways.get();
        }
        // The caller's size decides how much of the answer it receives; the rest of its structure
        // is left as it was.
        let want = (out.struct_size as usize).min(std::mem::size_of::<StatsV1>());
        let mut whole = sum;
        whole.struct_size = want as u32;
        unsafe {
            std::ptr::copy_nonoverlapping(
                &whole as *const StatsV1 as *const u8,
                out as *mut StatsV1 as *mut u8,
                want,
            )
        };
    }
}

impl Drop for ChannelStatsHandle {
    fn drop(&mut self) {
        let mut guard = self.runtime.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let stats = &self.stats;
        guard.1.calls_started += stats.calls_started.get();
        guard.1.calls_completed += stats.calls_completed.get();
        guard.1.calls_failed += stats.calls_failed.get();
        guard.1.dials += stats.dials.get();
        guard.1.dial_failures += stats.dial_failures.get();
        guard.1.retries += stats.retries.get();
        guard.1.stream_resets += stats.stream_resets.get();
        guard.1.goaways += stats.goaways.get();
        // Pruned here, not at each read, so that a read is a walk over live channels.
        guard.0.retain(|channel| channel.strong_count() > 0 && !std::ptr::eq(channel.as_ptr(), Arc::as_ptr(stats)));
    }
}
