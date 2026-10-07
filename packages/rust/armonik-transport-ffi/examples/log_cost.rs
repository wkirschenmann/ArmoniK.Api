//! What an event costs a host that registered a log callback, in nanoseconds: one the filter
//! rejects, one it admits and a callback that only counts, and what creating a runtime adds.
//!
//! cargo run --release -p armonik-transport-ffi --example log_cost [iterations] [repetitions]
//!
//! Each figure is the median of the repetitions net of the empty loop, with its minimum and its
//! maximum, since a loaded machine moves the median by itself.

use std::ffi::c_void;
use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use armonik_transport_ffi::*;

static DELIVERED: AtomicU64 = AtomicU64::new(0);

unsafe extern "C" fn count(_: *mut c_void, record: *const ak_log_record) {
    // Counts, and reads the level, as a host that copies nothing would.
    black_box(unsafe { (*record).level });
    DELIVERED.fetch_add(1, Ordering::Relaxed);
}

unsafe extern "C" fn runtime_event(_: *mut c_void, _: *mut c_void, _: *const ak_event, _: usize) {}

fn bytes(text: &str) -> ak_bytes_in {
    ak_bytes_in {
        ptr: text.as_ptr(),
        len: text.len(),
    }
}

#[inline(never)]
fn empty(n: u64) -> u64 {
    let mut sum = 0;
    for i in 0..n {
        sum += black_box(i);
    }
    sum
}

#[inline(never)]
fn rejected_by_level(n: u64) -> u64 {
    let mut sum = 0;
    for i in 0..n {
        sum += black_box(i);
        tracing::trace!(target: "armonik_transport::bench", attempt = i, "never delivered");
    }
    sum
}

#[inline(never)]
fn rejected_by_target(n: u64) -> u64 {
    let mut sum = 0;
    for i in 0..n {
        sum += black_box(i);
        tracing::info!(target: "h2::bench", attempt = i, "never delivered");
    }
    sum
}

#[inline(never)]
fn delivered_message_only(n: u64) -> u64 {
    let mut sum = 0;
    for i in 0..n {
        sum += black_box(i);
        tracing::info!(target: "armonik_transport::bench", "delivered");
    }
    sum
}

#[inline(never)]
fn delivered_three_fields(n: u64) -> u64 {
    let mut sum = 0;
    for i in 0..n {
        sum += black_box(i);
        tracing::info!(
            target: "armonik_transport::bench",
            attempt = i,
            endpoint = "http://127.0.0.1:5001",
            ok = true,
            "dial started"
        );
    }
    sum
}

fn create(callback: ak_log_callback) -> (ak_handle, std::time::Duration) {
    let config = ak_runtime_config {
        struct_size: std::mem::size_of::<ak_runtime_config>() as u32,
        version: 0,
        flags: 0,
        reserved: 0,
        memory_ceiling: 0,
        memory_hard_ceiling: 0,
        channel_defaults_json: bytes(""),
        log_callback: callback,
        log_ctx: std::ptr::null_mut(),
    };
    let mut runtime = AK_HANDLE_NONE;
    let start = Instant::now();
    let status = unsafe {
        ak_runtime_create(
            &config,
            Some(runtime_event),
            std::ptr::null_mut(),
            &mut runtime,
            std::ptr::null_mut(),
        )
    };
    let took = start.elapsed();
    assert_eq!(status, ak_status::AK_STATUS_OK);
    (runtime, took)
}

fn destroy(runtime: ak_handle) {
    unsafe { ak_runtime_begin_shutdown(runtime, std::ptr::null_mut()) };
    while ak_runtime_status(runtime) != ak_runtime_state::AK_RUNTIME_QUIESCENT {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(
        unsafe { ak_runtime_destroy(runtime, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
}

fn main() {
    let iterations: u64 = std::env::args()
        .nth(1)
        .map_or(10_000_000, |value| value.parse().expect("a count"));
    let repetitions: usize = std::env::args()
        .nth(2)
        .map_or(9, |value| value.parse().expect("a count"));

    // Creation: the first installs the subscriber, the others do not; without a callback nothing
    // is kept and the filter selects nothing.
    let mut with = Vec::new();
    let mut without = Vec::new();
    for round in 0..repetitions {
        let (runtime, took) = create(None);
        destroy(runtime);
        let (logged, took_logged) = create(Some(count));
        destroy(logged);
        if round > 0 {
            without.push(took.as_secs_f64() * 1e3);
            with.push(took_logged.as_secs_f64() * 1e3);
        }
    }
    let median = |values: &mut Vec<f64>| {
        values.sort_by(f64::total_cmp);
        (
            values[0],
            values[values.len() / 2],
            values[values.len() - 1],
        )
    };
    let (min, mid, max) = median(&mut without);
    println!("ak_runtime_create, no callback   : ms min {min:.2} median {mid:.2} max {max:.2}");
    let (min, mid, max) = median(&mut with);
    println!("ak_runtime_create, with callback : ms min {min:.2} median {mid:.2} max {max:.2}");

    let (runtime, _) = create(Some(count));

    let time = |work: fn(u64) -> u64| {
        let mut samples: Vec<f64> = (0..repetitions)
            .map(|_| {
                let start = Instant::now();
                black_box(work(black_box(iterations)));
                start.elapsed().as_secs_f64() * 1e9 / iterations as f64
            })
            .collect();
        samples.sort_by(f64::total_cmp);
        samples
    };
    black_box(rejected_by_level(1000));
    black_box(rejected_by_target(1000));
    black_box(delivered_message_only(1000));
    black_box(delivered_three_fields(1000));
    let base = time(empty);
    let base = base[base.len() / 2];

    for (name, work, iterations_of) in [
        (
            "rejected by level (trace)   ",
            rejected_by_level as fn(u64) -> u64,
            iterations,
        ),
        (
            "rejected by target (h2 info)",
            rejected_by_target,
            iterations,
        ),
        (
            "delivered, message only     ",
            delivered_message_only,
            iterations / 10,
        ),
        (
            "delivered, three fields     ",
            delivered_three_fields,
            iterations / 10,
        ),
    ] {
        let before = DELIVERED.load(Ordering::Relaxed);
        let samples: Vec<f64> = {
            let mut samples: Vec<f64> = (0..repetitions)
                .map(|_| {
                    let start = Instant::now();
                    black_box(work(black_box(iterations_of)));
                    start.elapsed().as_secs_f64() * 1e9 / iterations_of as f64
                })
                .collect();
            samples.sort_by(f64::total_cmp);
            samples
        };
        let delivered = DELIVERED.load(Ordering::Relaxed) - before;
        println!(
            "{name}: ns/event net of the loop: min {:.1} median {:.1} max {:.1} (delivered {delivered})",
            samples[0] - base,
            samples[samples.len() / 2] - base,
            samples[samples.len() - 1] - base,
        );
    }
    destroy(runtime);
}
