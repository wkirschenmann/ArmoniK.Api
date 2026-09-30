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

/// An end of the sending that comes while a send is on its way to the queue waits for it rather than
/// going ahead of it: ahead, the writer would drop the send half first and acquit a message that
/// never left.
#[test]
fn an_end_of_the_sending_does_not_overtake_a_send_being_queued() {
    let server = TestServer::start();
    let host = Host::start();
    let channel = host.channel(&server.endpoint);
    let call = start_call(channel, ECHO, &[]);

    let (reached, reaching) = mpsc::channel::<()>();
    let (release, released) = mpsc::channel::<()>();
    let (reached, released) = (Mutex::new(reached), Mutex::new(released));
    hooks::before_queueing(Some(Arc::new(move || {
        let _ = reached.lock().expect("one send").send(());
        let _ = released.lock().expect("one send").recv();
    })));
    let _unhook = Unhook;

    // Lent and filled on the sending thread: the buffer's pointers stay on its thread.
    let sender = std::thread::spawn(move || {
        let (status, buffer) = lend(call, 5);
        assert_eq!(status, ak_status::AK_STATUS_OK);
        unsafe { std::ptr::copy_nonoverlapping(b"hello".as_ptr(), buffer.ptr, 5) };
        unsafe { ak_call_send_message(call, buffer) }
    });
    reaching
        .recv_timeout(Duration::from_secs(10))
        .expect("the send reaches the queue");

    let ender = std::thread::spawn(move || ak_call_end_send(call));
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        !ender.is_finished(),
        "the end did not wait for the send being queued"
    );

    release.send(()).expect("the send is waiting");
    assert_eq!(
        sender.join().expect("the send returns"),
        ak_status::AK_STATUS_OK
    );
    assert_eq!(
        ender.join().expect("the end returns"),
        ak_status::AK_STATUS_OK
    );

    let seen = host.recorder.await_terminal();
    assert_eq!(seen.status_code(), Some(0), "{}", seen.status_message());
    assert_eq!(seen.message_payloads(), vec![b"hello".to_vec()]);

    host.recorder.consume_all();
    support::await_call_reclaimed(call);
    host.stop();
}

/// Takes the hooks away however the test ends: they are process-wide, and the next test in this
/// binary would otherwise run into them.
struct Unhook;

impl Drop for Unhook {
    fn drop(&mut self) {
        hooks::before_charge(None);
        hooks::before_queueing(None);
    }
}
