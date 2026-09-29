//! Interleavings the scheduler reaches only by chance, held open with the crate's test hooks.

// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use armonik_transport_ffi::hooks;
use armonik_transport_ffi::*;
use support::host::*;
use support::{TestServer, ECHO};

/// A lend that passed its checks before the shutdown began, and charges the ledger only after the
/// shutdown found nothing owed, lends nothing: QUIESCENT is the promise that no buffer is out.
#[test]
fn a_lend_the_shutdown_did_not_count_is_refused_rather_than_outliving_quiescence() {
    let server = TestServer::start();
    let host = Host::start();
    let channel = host.channel(&server.endpoint);
    let call = start_call(channel, ECHO, &[]);

    let (reached, reaching) = mpsc::channel::<()>();
    let (release, released) = mpsc::channel::<()>();
    let (reached, released) = (Mutex::new(reached), Mutex::new(released));
    hooks::before_charge(Some(Arc::new(move || {
        let _ = reached.lock().expect("one lend").send(());
        let _ = released.lock().expect("one lend").recv();
    })));
    let _unhook = Unhook;

    // The status and whether a buffer came back: the buffer's pointers stay on its thread.
    let lender = std::thread::spawn(move || {
        let (status, buffer) = lend(call, 8);
        (status, buffer.owner.is_null())
    });
    // Bounded: the sender lives in the hook, so a lend refused before it would never disconnect.
    reaching
        .recv_timeout(Duration::from_secs(10))
        .expect("the lend reaches the charge");

    host.stop();
    release.send(()).expect("the lend is waiting");
    let (status, nothing_lent) = lender.join().expect("the lend returns");

    assert_eq!(
        status,
        ak_status::AK_STATUS_INVALID_STATE,
        "a buffer was lent after the runtime reported QUIESCENT"
    );
    assert!(nothing_lent);
    assert_eq!(memory_usage(host.runtime).bytes_used, 0);
}

/// Takes the hook away however the test ends: it is process-wide, and the next test in this binary
/// would otherwise run into it.
struct Unhook;

impl Drop for Unhook {
    fn drop(&mut self) {
        hooks::before_charge(None);
    }
}
