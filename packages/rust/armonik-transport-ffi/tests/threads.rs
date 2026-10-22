//! The library's own threads, counted with the crate's test hooks.

// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use armonik_transport_ffi::hooks;
use armonik_transport_ffi::*;
use support::host::*;
use support::{TestServer, ECHO};

/// The workers start as connections are given to them, never more than `WorkerThreads`, and
/// outlive the channel whose connections started them: none ends with it, and the next channel's
/// calls run on them. The teardown ends them all before it reports QUIESCENT, which promises that
/// no thread of the runtime is left.
#[test]
fn workers_start_with_connections_up_to_their_count_and_none_outlives_quiescence() {
    let server = TestServer::start();
    let host = Host::with_runtime_options(r#"{"WorkerThreads":{"Count":2}}"#);
    assert_eq!(
        hooks::worker_threads(),
        1,
        "the first starts with the runtime"
    );
    // Slower to end than the rest of the teardown, so that QUIESCENT is early unless it waits.
    let ended = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&ended);
    hooks::worker_ending(Some(Arc::new(move || {
        counted.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(200))
    })));
    let _unhook = Unhook;

    // A connection for each call, each call held open: three connections.
    let channel = host.channel_with(
        &server.endpoint,
        r#"{"Http2":{"SimultaneousCallsPerConnection":{"Limit":1}}}"#,
    );
    let calls: Vec<ak_handle> = (0..3).map(|_| start_call(channel, ECHO, &[])).collect();
    support::poll_until(
        || hooks::worker_threads() == 2,
        || format!("{} worker threads", hooks::worker_threads()),
    );
    for call in calls {
        unsafe { ak_call_cancel(call, std::ptr::null_mut()) };
    }
    host.recorder.await_terminals(3);
    ak_channel_release(channel);

    let next = host.channel(&server.endpoint);
    send_one(start_call(next, ECHO, &[]), b"after");
    host.recorder.await_terminals(4);
    let last = host
        .recorder
        .last_of(ak_event_kind::AK_EVENT_STATUS)
        .expect("a terminal");
    assert_eq!(last.status_code, 0, "the next channel's call succeeds");
    assert_eq!(
        ended.load(Ordering::SeqCst),
        0,
        "a worker ended with its channel"
    );
    assert_eq!(
        hooks::worker_threads(),
        2,
        "the next channel runs on the same workers"
    );
    ak_channel_release(next);

    host.stop();
    assert_eq!(hooks::worker_threads(), 0);
}

struct Unhook;

impl Drop for Unhook {
    fn drop(&mut self) {
        hooks::worker_ending(None);
    }
}
