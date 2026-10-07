//! Point 6: a C surface over the prototypes, so that a .NET host can register a log callback and
//! read statistics, and the cost of the crossing can be measured from the managed side.

use std::ffi::c_void;
use std::sync::Arc;
use std::time::Instant;

use crate::record::LogCallback;
use crate::runtime::{Front, ObsRuntime};
use crate::stats::{ChannelStatsHandle, RuntimeStats, StatsV1};

pub struct SpikeRuntime {
    runtime: ObsRuntime,
    stats: Arc<RuntimeStats>,
    channel: ChannelStatsHandle,
}

#[no_mangle]
pub extern "C" fn spike_runtime_new() -> *mut SpikeRuntime {
    let stats = Arc::new(RuntimeStats::default());
    let channel = stats.open_channel();
    Box::into_raw(Box::new(SpikeRuntime {
        runtime: ObsRuntime::new(Front::Layered),
        stats,
        channel,
    }))
}

/// # Safety
/// `runtime` comes from `spike_runtime_new`, once.
#[no_mangle]
pub unsafe extern "C" fn spike_runtime_free(runtime: *mut SpikeRuntime) {
    drop(unsafe { Box::from_raw(runtime) });
}

/// # Safety
/// `runtime` is live; `filter` names `filter_len` bytes.
#[no_mangle]
pub unsafe extern "C" fn spike_set_log_callback(
    runtime: *mut SpikeRuntime,
    callback: LogCallback,
    ctx: *mut c_void,
    filter: *const u8,
    filter_len: usize,
) -> i32 {
    let runtime = unsafe { &*runtime };
    let filter = unsafe { std::slice::from_raw_parts(filter, filter_len) };
    let Ok(filter) = std::str::from_utf8(filter) else {
        return 3;
    };
    match runtime.runtime.obs.set_log_callback(callback, ctx, filter) {
        Ok(()) => 0,
        Err(_) => 3,
    }
}

static NATIVE_COUNT: crate::testkit::Counter = crate::testkit::Counter {
    count: std::sync::atomic::AtomicU64::new(0),
};

/// Registers a callback of this library's own that only counts: the floor a managed callback is
/// measured against.
///
/// # Safety
/// `runtime` is live.
#[no_mangle]
pub unsafe extern "C" fn spike_set_native_count(runtime: *mut SpikeRuntime) -> i32 {
    let runtime = unsafe { &*runtime };
    match runtime
        .runtime
        .obs
        .set_log_callback(crate::testkit::count, NATIVE_COUNT.ctx(), "")
    {
        Ok(()) => 0,
        Err(_) => 3,
    }
}

/// # Safety
/// `runtime` is live.
#[no_mangle]
pub unsafe extern "C" fn spike_clear_log_callback(runtime: *mut SpikeRuntime) -> i32 {
    let runtime = unsafe { &*runtime };
    match runtime.runtime.obs.clear_log_callback() {
        Ok(()) => 0,
        Err(_) => 4,
    }
}

/// Runs `threads` library threads, each logging `events` records of three fields, and returns the
/// time it took, in nanoseconds.
///
/// # Safety
/// `runtime` is live.
#[no_mangle]
pub unsafe extern "C" fn spike_emit(runtime: *mut SpikeRuntime, threads: u32, events: u64) -> u64 {
    let runtime = unsafe { &*runtime };
    let dispatch = runtime.runtime.dispatch.clone();
    let start = Instant::now();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            let dispatch = dispatch.clone();
            scope.spawn(move || {
                tracing::dispatcher::with_default(&dispatch, || {
                    for attempt in 0..events {
                        tracing::info!(
                            target: "armonik_transport::grpc::channel",
                            endpoint = "http://10.0.0.1:5001",
                            attempt,
                            "dial started"
                        );
                    }
                });
            });
        }
    });
    start.elapsed().as_nanos() as u64
}

/// Calls `callback` `n` times in a tight loop with one fixed record of two fields, and returns the
/// time in nanoseconds: the cost of the crossing and of the callback, without rendering an event.
///
/// # Safety
/// `callback` is callable.
#[no_mangle]
pub unsafe extern "C" fn spike_invoke_callback(callback: LogCallback, n: u64) -> u64 {
    use crate::record::{BytesIn, LogField, LogRecord};
    let text = |s: &'static str| BytesIn {
        ptr: s.as_ptr(),
        len: s.len(),
    };
    let fields = [
        LogField {
            key: text("endpoint"),
            value: text("http://10.0.0.1:5001"),
        },
        LogField {
            key: text("attempt"),
            value: text("17"),
        },
    ];
    let record = LogRecord {
        struct_size: std::mem::size_of::<LogRecord>() as u32,
        level: 3,
        field_count: 2,
        reserved: 0,
        target: text("armonik_transport::grpc::channel"),
        message: text("dial started"),
        fields: fields.as_ptr(),
    };
    let start = Instant::now();
    for _ in 0..n {
        unsafe { callback(std::ptr::null_mut(), &record) };
    }
    start.elapsed().as_nanos() as u64
}

/// Moves the numbers a host will read.
///
/// # Safety
/// `runtime` is live.
#[no_mangle]
pub unsafe extern "C" fn spike_stats_bump(runtime: *mut SpikeRuntime, dials: u64, in_flight: u64) {
    let runtime = unsafe { &*runtime };
    runtime.channel.stats.dials.add(dials);
    runtime.channel.stats.calls_in_flight.add(in_flight);
}

/// # Safety
/// `runtime` is live; `out` is a `StatsV1` whose `struct_size` the caller set.
#[no_mangle]
pub unsafe extern "C" fn spike_stats_read(runtime: *mut SpikeRuntime, out: *mut StatsV1) {
    let runtime = unsafe { &*runtime };
    runtime.stats.read(unsafe { &mut *out });
}
