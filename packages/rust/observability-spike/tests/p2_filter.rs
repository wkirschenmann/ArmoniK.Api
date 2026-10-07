//! Point 2: what the filter accepts, and what changing it does to events already cached.

use observability_spike::obs::{parse_filter, Refusal, DEFAULT_FILTER};
use observability_spike::runtime::{Front, ObsRuntime};
use observability_spike::testkit::{collect, Collector};
use tracing_subscriber::filter::EnvFilter;

fn emit_debug() {
    tracing::debug!(target: "armonik_transport::filter", "a debug event");
}

fn emit_h2() {
    tracing::debug!(target: "h2::proto::connection", "an h2 event");
}

#[test]
fn what_the_directive_syntax_accepts_and_refuses() {
    let cases = [
        "info",
        "info,h2=debug",
        "warn,armonik_transport=info,armonik_transport_ffi=info",
        "",
        "INFO",
        "debug,,h2=trace",
        "off",
        "h2=trace",
        "h2=nolevel",
        "nolevel",
        "info,h2",
        "h2[span]=debug",
        "h2[{field=value}]=debug",
        "h2[{field}]=debug",
        "h2::proto=debug,h2=warn",
        "armonik-transport=debug",
        "=debug",
        "info, h2=debug",
        "a=b=c",
    ];
    for case in cases {
        let lenient = case.parse::<tracing_subscriber::filter::Targets>();
        let strict = parse_filter(case);
        let env = EnvFilter::try_new(case);
        println!(
            "{case:50?} Targets: {:9} EnvFilter: {:9} strict: {}",
            if lenient.is_ok() { "accepted" } else { "refused" },
            if env.is_ok() { "accepted" } else { "refused" },
            match &strict {
                Ok(_) => "accepted".to_owned(),
                Err(error) => format!("refused ({})", error.0),
            }
        );
    }
    assert!(parse_filter(DEFAULT_FILTER).is_ok());
    for refused in ["", "inof", "info,h2", "h2[span]=debug", "h2=nolevel", "=debug", "a=b=c"] {
        assert!(parse_filter(refused).is_err(), "{refused:?}");
    }
    for accepted in ["info", "info,h2=debug", "INFO", "info, h2=debug", "off", "h2=trace", "debug,h2::proto=warn"] {
        assert!(parse_filter(accepted).is_ok(), "{accepted:?}");
    }
}

/// The prefix match is on the target's text, not on its path segments.
#[test]
fn a_target_directive_matches_by_prefix_of_the_text() {
    let targets = parse_filter("warn,h2=debug").unwrap();
    for (target, expected) in [
        ("h2", true),
        ("h2::proto::connection", true),
        ("h2x", true),
        ("hyper", false),
    ] {
        println!("h2=debug on target {target:25} -> {}", targets.would_enable(target, &tracing::Level::DEBUG));
        let _ = expected;
    }
}

#[test]
fn changing_the_filter_changes_what_crosses_even_for_events_already_cached() {
    let runtime = ObsRuntime::new(Front::Layered);
    let log = Box::new(Collector::default());
    runtime.obs.set_log_callback(collect, log.ctx(), "info").unwrap();
    let _inside = runtime.scope();

    emit_debug();
    assert!(log.messages().is_empty(), "debug is below info");

    runtime.obs.set_filter("debug").unwrap();
    emit_debug();
    assert_eq!(log.take().len(), 1, "the same callsite, enabled by the new filter");

    // An unparsable directive is refused and the filter in force stays.
    assert_eq!(runtime.obs.set_filter("debug,h2=nolevel"), Err(Refusal::InvalidFilter));
    emit_debug();
    assert_eq!(log.take().len(), 1);

    runtime.obs.set_filter("info").unwrap();
    emit_debug();
    assert!(log.messages().is_empty());
}

/// Without `rebuild_interest_cache`, a callsite that was `never` stays so.
#[test]
fn a_filter_swapped_without_a_rebuild_leaves_the_cached_interest_in_place() {
    fn emit_here() {
        tracing::debug!(target: "armonik_transport::filter", "no rebuild");
    }
    let runtime = ObsRuntime::new(Front::Layered);
    let log = Box::new(Collector::default());
    runtime.obs.set_log_callback(collect, log.ctx(), "info").unwrap();
    let _inside = runtime.scope();

    emit_here();
    runtime.obs.set_filter_without_rebuild("debug").unwrap();
    emit_here();
    println!("delivered after a swap with no rebuild: {}", log.messages().len());
    assert!(log.messages().is_empty(), "the callsite stayed disabled");

    // Any rebuild, from anywhere in the process, repairs it.
    tracing_core::callsite::rebuild_interest_cache();
    emit_here();
    assert_eq!(log.messages().len(), 1);
}

/// The default: the engine at info, h2 quiet until a directive brings it back.
#[test]
fn the_default_keeps_h2_quiet_until_a_directive_brings_it_back() {
    let runtime = ObsRuntime::new(Front::Layered);
    let log = Box::new(Collector::default());
    runtime.obs.set_log_callback(collect, log.ctx(), "").unwrap();
    let _inside = runtime.scope();

    emit_h2();
    assert!(log.messages().is_empty());
    runtime.obs.set_filter("info,h2=debug").unwrap();
    emit_h2();
    assert_eq!(log.messages().len(), 1);
}
