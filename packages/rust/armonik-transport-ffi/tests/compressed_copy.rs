//! The compressed copy of a message counts against the memory ceiling, and a copy the ceiling has
//! no room for is dropped: the message goes out as the host wrote it, flagged uncompressed, and
//! the call succeeds.

// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use armonik_transport_ffi::hooks;
use armonik_transport_ffi::*;
use support::host::*;
use support::{TestServer, FRAMES};

const CEILING: usize = 64 * 1024;
const LEN: usize = 40_000;
const GZIP: &str = r#"{"Grpc":{"Send":{"Compression":"Gzip"}}}"#;

/// What gzip makes of the message: the engine compresses with the same encoder at the same level.
fn compressed_len(message: &[u8]) -> usize {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(message).expect("into a vector");
    encoder.finish().expect("a gzip stream").len()
}

fn message() -> Vec<u8> {
    b"abc".repeat(LEN / 3)
}

/// The hook is process-wide, and a test makes a runtime for each shape: the tests take turns.
static TURN: Mutex<()> = Mutex::new(());

/// Takes the hook away however the test ends: it is process-wide.
struct Unhook;

impl Drop for Unhook {
    fn drop(&mut self) {
        hooks::before_copy_charge(None);
    }
}

/// What the server says of the one request a call sends, as a call that declared one request or
/// as a stream: `encoding=gzip frames=<flag>:<length>`.
fn send_one(host: &Host, channel: ak_handle, written: &[u8], one_request: bool) -> String {
    let call = if one_request {
        start_call_flagged(channel, FRAMES, &[], AK_CALL_ONE_REQUEST)
    } else {
        start_call(channel, FRAMES, &[])
    };
    let (status, buffer) = lend(call, written.len());
    assert_eq!(status, ak_status::AK_STATUS_OK);
    unsafe { std::ptr::copy_nonoverlapping(written.as_ptr(), buffer.ptr, written.len()) };
    assert_eq!(
        unsafe { ak_call_send_message(call, buffer, written.len(), std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    if !one_request {
        host.recorder.await_write_done();
        assert_eq!(
            unsafe { ak_call_end_send(call, std::ptr::null_mut()) },
            ak_status::AK_STATUS_OK
        );
    }
    let seen = host.recorder.await_terminal();
    assert_eq!(seen.status_code(), Some(0), "{}", seen.status_message());
    let said = String::from_utf8(seen.message_payloads().concat()).expect("text");
    host.recorder.consume_all();
    support::await_call_reclaimed(call);
    said
}

/// A copy the ceiling has room for goes out compressed, counted while it is held and given back
/// with the message.
#[test]
fn a_copy_that_fits_is_sent_compressed() {
    let _turn = TURN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for one_request in [true, false] {
        let server = TestServer::start();
        let host = Host::with_ceiling(CEILING as u64);
        let channel = host.channel_with(&server.endpoint, GZIP);
        let written = message();

        let said = send_one(&host, channel, &written, one_request);

        let payload = compressed_len(&written);
        assert_eq!(
            said,
            format!("encoding=gzip frames=1:{payload}"),
            "one request: {one_request}"
        );
        assert_eq!(memory_usage(host.runtime).bytes_used, 0);
        ak_channel_release(channel);
        host.stop();
    }
}

/// A copy the ceiling has no room for is dropped: the server receives the whole message, flagged
/// uncompressed under the call's own `grpc-encoding`, and the call succeeds.
#[test]
fn a_copy_that_does_not_fit_is_sent_uncompressed() {
    let _turn = TURN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for one_request in [true, false] {
        let server = TestServer::start();
        let host = Host::with_ceiling(CEILING as u64);
        let channel = host.channel_with(&server.endpoint, GZIP);
        let other = start_call(channel, FRAMES, &[]);
        let written = message();
        let framed = 5 + compressed_len(&written);

        // Another call takes what leaves the copy one byte short, when the copy asks.
        let taken = Arc::new(Mutex::new(None));
        let once = Arc::new(AtomicBool::new(false));
        let hook_taken = Arc::clone(&taken);
        let runtime = host.runtime;
        hooks::before_copy_charge(Some(Arc::new(move || {
            if !once.swap(true, Ordering::SeqCst) {
                let used = memory_usage(runtime).bytes_used as usize;
                let (status, buffer) = lend(other, CEILING - used - (framed - 1));
                assert_eq!(status, ak_status::AK_STATUS_OK);
                *hook_taken.lock().unwrap() =
                    Some((buffer.ptr as usize, buffer.len, buffer.owner as usize));
            }
        })));
        let unhook = Unhook;

        let said = send_one(&host, channel, &written, one_request);
        drop(unhook);

        assert_eq!(
            said,
            format!("encoding=gzip frames=0:{}", written.len()),
            "one request: {one_request}"
        );
        let (ptr, len, owner) = taken.lock().unwrap().take().expect("the hook ran");
        unsafe {
            ak_return_call_buffer(ak_buffer {
                ptr: ptr as *mut u8,
                len,
                owner: owner as *mut std::ffi::c_void,
            })
        };
        assert_eq!(memory_usage(host.runtime).bytes_used, 0);
        assert_eq!(
            unsafe { ak_call_cancel(other, std::ptr::null_mut()) },
            ak_status::AK_STATUS_OK
        );
        host.recorder.await_terminals(2);
        host.recorder.consume_all();
        support::await_call_reclaimed(other);
        ak_channel_release(channel);
        host.stop();
    }
}
