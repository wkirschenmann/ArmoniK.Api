//! Point 2: what `ak_runtime_set_log_filter` costs, by the callsites registered and the
//! dispatchers alive.
//!
//! filter_change <dispatchers> <callsites: 0|256|4352> [front: layered|envg]

use std::time::Instant;

use observability_spike::fronts;
use observability_spike::obs::{bare_dispatch, layered_dispatch, RtObs};
use observability_spike::testkit::{count, Counter};
use tracing::Dispatch;
use tracing_subscriber::filter::EnvFilter;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dispatchers: usize = args[1].parse().unwrap();
    let callsites: usize = args[2].parse().unwrap();
    let front = args.get(3).map_or("layered", String::as_str);

    let counter: &'static Counter = Box::leak(Box::new(Counter::default()));
    observability_spike::runtime::install_sentinel();

    let mut obses = Vec::new();
    let mut handles = Vec::new();
    let mut dispatches: Vec<Dispatch> = Vec::new();
    for _ in 0..dispatchers {
        match front {
            "layered" | "bare" => {
                let obs = RtObs::new(1);
                obs.set_log_callback(count, counter.ctx(), "debug").unwrap();
                dispatches.push(if front == "bare" {
                    bare_dispatch(&obs)
                } else {
                    layered_dispatch(&obs)
                });
                obses.push(obs);
            }
            "envg" => {
                let (dispatch, handle) = fronts::env_global("debug", count, counter.ctx());
                dispatches.push(dispatch);
                handles.push(handle);
            }
            other => panic!("front {other}"),
        }
    }
    let _guard = tracing::dispatcher::set_default(&dispatches[0]);

    // Registers the callsites while debug is enabled, then changes the filter back and forth.
    match callsites {
        0 => {}
        256 => observability_spike::bulk::touch_256(),
        _ => observability_spike::bulk::touch_4096(),
    }

    let change = |directives: &str| match front {
        "envg" => handles[0].set(directives),
        _ => obses[0].set_filter(directives).unwrap(),
    };
    let mut samples = Vec::new();
    for round in 0..200 {
        let directives = if round % 2 == 0 { "info,h2=debug" } else { "debug" };
        let start = Instant::now();
        change(directives);
        samples.push(start.elapsed().as_secs_f64() * 1e6);
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "{front:8} dispatchers={dispatchers:2} callsites={callsites:5} | set_filter us: min {:8.1}  median {:8.1}  p95 {:8.1}  max {:8.1}",
        samples[0],
        samples[100],
        samples[190],
        samples[199]
    );

    // The parse alone.
    let start = Instant::now();
    for _ in 0..1000 {
        std::hint::black_box(observability_spike::obs::parse_filter("info,h2=debug,hyper=warn").unwrap());
    }
    let targets_us = start.elapsed().as_secs_f64() * 1e3;
    let start = Instant::now();
    for _ in 0..1000 {
        std::hint::black_box(EnvFilter::try_new("info,h2=debug,hyper=warn").unwrap());
    }
    let env_us = start.elapsed().as_secs_f64() * 1e3;
    println!("         parse: strict+Targets {targets_us:.2} us, EnvFilter {env_us:.2} us");
}
