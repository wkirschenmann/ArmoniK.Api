//! Scratch instrumentation: the timestamps of one sequential call's checkpoints, in the
//! performance counter's ticks, which is the clock .NET's Stopwatch reads.

use std::sync::atomic::{AtomicI64, Ordering};

pub const POINTS: usize = 24;

static AT: [AtomicI64; POINTS] = [const { AtomicI64::new(0) }; POINTS];

#[cfg(windows)]
fn now() -> i64 {
    #[link(name = "kernel32")]
    extern "system" {
        fn QueryPerformanceCounter(count: *mut i64) -> i32;
    }
    let mut count = 0i64;
    unsafe { QueryPerformanceCounter(&mut count) };
    count
}

#[cfg(not(windows))]
fn now() -> i64 {
    0
}

pub fn mark(point: usize) {
    AT[point].store(now(), Ordering::Relaxed);
}

pub fn mark_first(point: usize) {
    let _ = AT[point].compare_exchange(0, now(), Ordering::Relaxed, Ordering::Relaxed);
}

pub fn at(point: usize) -> i64 {
    AT.get(point).map_or(0, |at| at.load(Ordering::Relaxed))
}

pub fn reset() {
    for at in &AT {
        at.store(0, Ordering::Relaxed);
    }
}
