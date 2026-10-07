//! Point 2: what a disabled (and an enabled) event costs in a hot loop, by the dispatchers the
//! process holds.
//!
//! cost <front> <filters> <global> <emit> [iterations] [repetitions]
//!   front    bare | layered | envg | targg | envp
//!   filters  one filter per runtime, joined by '+'; the measuring thread is under the first;
//!            '-' for no runtime (the thread is under no dispatcher of its own)
//!   global   a filter for a process-wide subscriber, or '-'
//!   emit     debug | info (an event at that level, three fields)
//!
//! The callsite cache is process-wide, so each combination runs in a process of its own.

use std::hint::black_box;
use std::sync::atomic::Ordering;
use std::time::Instant;

use observability_spike::fronts;
use observability_spike::obs::{bare_dispatch, layered_dispatch, RtObs};
use observability_spike::testkit::{count, Counter};
use tracing::Dispatch;

#[inline(never)]
fn baseline(n: u64) -> u64 {
    let mut sum = 0;
    for i in 0..n {
        sum += black_box(i);
    }
    sum
}

#[inline(never)]
fn hot_debug(n: u64) -> u64 {
    let mut sum = 0;
    for i in 0..n {
        sum += black_box(i);
        tracing::debug!(target: "armonik_transport::hot", attempt = i, endpoint = "http://127.0.0.1:5001", "dial started");
    }
    sum
}

#[inline(never)]
fn hot_info(n: u64) -> u64 {
    let mut sum = 0;
    for i in 0..n {
        sum += black_box(i);
        tracing::info!(target: "armonik_transport::hot", attempt = i, endpoint = "http://127.0.0.1:5001", "dial started");
    }
    sum
}

fn dispatch_of(front: &str, filter: &str, counter: &'static Counter) -> Dispatch {
    match front {
        "bare" | "layered" => {
            let obs = RtObs::new(1);
            obs.set_log_callback(count, counter.ctx(), filter).unwrap();
            if front == "bare" {
                bare_dispatch(&obs)
            } else {
                layered_dispatch(&obs)
            }
        }
        "envg" => fronts::env_global(filter, count, counter.ctx()).0,
        "targg" => fronts::targets_global(filter, count, counter.ctx()).0,
        "envp" => fronts::env_per_layer(filter, count, counter.ctx()).0,
        other => panic!("front {other}"),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let front = args[1].as_str();
    let filters: Vec<&str> = args[2].split('+').collect();
    let global = args[3].as_str();
    let emit = args[4].as_str();
    let iterations: u64 = args.get(5).map_or(20_000_000, |v| v.parse().unwrap());
    let repetitions: usize = args.get(6).map_or(7, |v| v.parse().unwrap());

    let counter: &'static Counter = Box::leak(Box::new(Counter::default()));

    if std::env::var("SENTINEL").is_ok_and(|v| v == "1") {
        observability_spike::runtime::install_sentinel();
    }
    if global != "-" {
        tracing::dispatcher::set_global_default(dispatch_of(front, global, counter)).unwrap();
    }

    let mut mine: Option<Dispatch> = None;
    let mut others = Vec::new();
    for (index, filter) in filters.iter().enumerate() {
        if *filter == "-" {
            continue;
        }
        let dispatch = dispatch_of(front, filter, counter);
        if index == 0 {
            mine = Some(dispatch);
        } else {
            others.push(dispatch);
        }
    }
    let _guard = mine.as_ref().map(tracing::dispatcher::set_default);

    let run = |f: fn(u64) -> u64| {
        let mut samples: Vec<f64> = (0..repetitions)
            .map(|_| {
                let start = Instant::now();
                black_box(f(black_box(iterations)));
                start.elapsed().as_secs_f64() * 1e9 / iterations as f64
            })
            .collect();
        samples.sort_by(f64::total_cmp);
        samples
    };
    let target: fn(u64) -> u64 = match emit {
        "debug" => hot_debug,
        "info" => hot_info,
        other => panic!("emit {other}"),
    };
    // Warm up so that the callsite is registered before it is timed.
    black_box(target(1000));
    let base = run(baseline);
    let timed = run(target);
    let net = |value: f64| value - base[base.len() / 2];
    println!(
        "{front:8} filters={:12} global={:6} emit={emit:5} | ns/event net of loop: min {:7.2}  median {:7.2}  max {:7.2} | delivered {}",
        args[2],
        global,
        net(timed[0]),
        net(timed[timed.len() / 2]),
        net(timed[timed.len() - 1]),
        counter.count.load(Ordering::Relaxed),
    );
    drop(others);
}
