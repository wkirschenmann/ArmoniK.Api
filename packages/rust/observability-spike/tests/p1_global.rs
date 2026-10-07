//! Point 1: a host's own global subscriber beside runtimes that carry their own.

use observability_spike::obs::{bare_dispatch, RtObs};
use observability_spike::runtime::{Front, ObsRuntime};
use observability_spike::testkit::{collect, Collector};

fn info_event(what: &'static str) {
    tracing::info!(target: "armonik_transport::global", "{what}");
}

fn warn_event(what: &'static str) {
    tracing::warn!(target: "armonik_transport::global", "{what}");
}

#[test]
fn a_global_subscriber_sees_what_no_runtime_thread_emits_and_nothing_else() {
    // The host's own subscriber: a filter of its own, `warn`.
    let host = RtObs::new(100);
    let host_log = Box::new(Collector::default());
    host.set_log_callback(collect, host_log.ctx(), "warn").unwrap();
    tracing::dispatcher::set_global_default(bare_dispatch(&host)).unwrap();

    let runtime = ObsRuntime::new(Front::Bare);
    let log = Box::new(Collector::default());
    runtime.obs.set_log_callback(collect, log.ctx(), "info").unwrap();
    let channel = runtime.start_channel_thread();

    // Outside any runtime: the host's subscriber decides.
    std::thread::spawn(|| {
        info_event("outside info");
        warn_event("outside warn");
    })
    .join()
    .unwrap();
    // On the runtime's thread: the runtime's alone, whatever the host's filter says.
    runtime
        .tokio
        .block_on(channel.handle.spawn(async {
            info_event("inside info");
            warn_event("inside warn");
        }))
        .unwrap();

    assert_eq!(host_log.messages(), ["outside warn"]);
    assert_eq!(log.messages(), ["inside info", "inside warn"]);
}
