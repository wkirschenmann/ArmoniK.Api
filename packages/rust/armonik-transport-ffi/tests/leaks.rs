//! Leak assertions: a call must give back everything it took.
//!
//! Their own test binary, and one at a time, because these are the only tests here that measure
//! *global* state — the shared tokio runtime's alive-task count and the process's OS handle count.
//! Left in with the others, they would be reading those counters while a dozen concurrent tests were
//! busy starting and cancelling calls of their own, and would fail on the noise rather than on a leak.

mod common;

use std::time::{Duration, Instant};

use bytes::Bytes;
use common::abi::{Client, Kind, StartOptions};
use common::server::{serve, TestService, METHOD_PATH};
use serial_test::serial;

/// gRPC's `CANCELLED`.
const GRPC_CANCELLED: i32 = 1;

#[test]
#[serial]
fn a_completed_call_leaves_no_task_behind() {
    let endpoint = serve(TestService::canned([Bytes::from_static(b"pong")]));
    let client = Client::to(&endpoint, &[]);

    assert_no_task_growth("completed calls", || {
        let call = client.start(METHOD_PATH, Kind::Unary, StartOptions::default());
        call.send_ok(b"ping");
        call.close_send_ok();
        let (_, outcome) = call.drain();
        assert_eq!(outcome.code, 0);
    });
}

#[test]
#[serial]
fn a_cancelled_call_leaves_no_task_behind() {
    // The case that actually leaks if cancellation is not wired all the way through: the server never
    // answers, so nothing else will ever end these tasks.
    let endpoint = serve(TestService::hang());
    let client = Client::to(&endpoint, &[]);

    assert_no_task_growth("cancelled calls", || {
        let call = client.start(METHOD_PATH, Kind::Unary, StartOptions::default());
        call.send_ok(b"ping");
        call.close_send_ok();
        call.cancel();
        let (_, outcome) = call.drain();
        assert_eq!(outcome.code, GRPC_CANCELLED);
    });
}

#[test]
#[serial]
fn a_call_cancelled_before_it_was_closed_leaves_no_task_behind() {
    // The buffering phase again, from the leak side: the driving task is parked waiting for a
    // `close_send` that never comes, and only the cancellation reaching that wait can end it.
    let endpoint = serve(TestService::hang());
    let client = Client::to(&endpoint, &[]);

    assert_no_task_growth("calls cancelled before being closed", || {
        let call = client.start(METHOD_PATH, Kind::Unary, StartOptions::default());
        call.send_ok(b"ping");
        call.cancel();
        let (_, outcome) = call.drain();
        assert_eq!(outcome.code, GRPC_CANCELLED);
    });
}

#[test]
#[serial]
fn a_call_abandoned_without_being_drained_leaves_no_task_behind() {
    // The worst case for a leak: freed while in flight, never drained, against a server that will
    // never answer. Nothing but `ak_call_free`'s own abort can end these.
    let endpoint = serve(TestService::hang());
    let client = Client::to(&endpoint, &[]);

    assert_no_task_growth("abandoned calls", || {
        let call = client.start(METHOD_PATH, Kind::Unary, StartOptions::default());
        call.send_ok(b"ping");
        drop(call);
    });
}

#[cfg(windows)]
#[test]
#[serial]
fn repeated_calls_do_not_leak_os_handles() {
    // Every call creates a Win32 event, and every one of them has to be closed exactly once — by
    // whichever of the driving task and `ak_call_free` finishes last. A miss here shows up as a
    // process that runs out of handles after a few million RPCs, which is a fortnight into
    // production, not during a test run.
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn GetProcessHandleCount(process: *mut std::ffi::c_void, count: *mut u32) -> i32;
    }

    fn handle_count() -> u32 {
        let mut count = 0u32;
        // SAFETY: a pseudo-handle for the current process, which needs no release, and a live local
        // to write the count into.
        let ok =
            unsafe { GetProcessHandleCount(GetCurrentProcess(), std::ptr::addr_of_mut!(count)) };
        assert_ne!(ok, 0, "GetProcessHandleCount failed");
        count
    }

    let endpoint = serve(TestService::canned([Bytes::from_static(b"pong")]));
    let client = Client::to(&endpoint, &[]);

    // Warm up, so the baseline includes the connection's own handles.
    for _ in 0..8 {
        let call = client.start(METHOD_PATH, Kind::Unary, StartOptions::default());
        call.send_ok(b"ping");
        call.close_send_ok();
        call.drain();
    }

    let baseline = handle_count();

    for _ in 0..200 {
        let call = client.start(METHOD_PATH, Kind::Unary, StartOptions::default());
        call.send_ok(b"ping");
        call.close_send_ok();
        call.drain();
    }

    let after = handle_count();
    // A small allowance: the runtime and the OS may hold a handle or two of their own, and the point
    // is that the count does not grow with the number of calls. Leaking one event per call would show
    // up here as +200.
    assert!(
        after <= baseline + 16,
        "200 calls grew the process handle count from {baseline} to {after}"
    );
}

/// Run `one_call` in two equal batches and assert the second leaves no more tasks alive than the
/// first.
///
/// Two batches rather than an absolute baseline, because that is what makes the assertion both strict
/// and stable: one-off costs — the tasks a connection spawns lazily the first time a stream is
/// cancelled, say — land in the first round and cancel out, while anything leaked *per call* grows
/// with the batch size and cannot hide.
fn assert_no_task_growth(what: &str, mut one_call: impl FnMut()) {
    const BATCH: usize = 32;

    for _ in 0..BATCH {
        one_call();
    }
    let first = settled_task_count();

    for _ in 0..BATCH {
        one_call();
    }
    let second = settled_task_count();

    assert!(
        second <= first,
        "{BATCH} more {what} grew the runtime from {first} to {second} alive task(s)"
    );
}

/// The alive-task count, once it has stopped moving.
///
/// A task is spawned before its first poll and reaped a little after its last, so reading the count
/// the instant a call returns is a race. Waiting for two identical readings is enough for a metric
/// that only decreases once the work is done.
fn settled_task_count() -> usize {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut previous = armonik_transport_ffi::runtime::alive_tasks();
    loop {
        std::thread::sleep(Duration::from_millis(20));
        let current = armonik_transport_ffi::runtime::alive_tasks();
        if current == previous {
            return current;
        }
        assert!(
            Instant::now() < deadline,
            "the runtime's task count never settled (last readings {previous} then {current})"
        );
        previous = current;
    }
}
