//! A unary call through the C ABI, against a real gRPC server.
//!
//! The test plays the host: it builds the structs the way C would, hands the library a callback,
//! and gives back everything the ABI says it owes. What it asserts is the event sequence, the
//! ownership ledger, and that the runtime reaches quiescence by itself once nothing is left out.

mod support;

use std::ffi::c_void;
use std::time::{Duration, Instant};

use armonik_transport_ffi::*;
use support::{blob, Recorder, Seen, TestServer, ECHO, FAIL};

/// Drives the ABI the way a binding would, and cleans up after itself.
struct Host {
    runtime: ak_handle,
    recorder: Box<Recorder>,
}

impl Host {
    fn start() -> Self {
        Self::with_ceiling(0)
    }

    fn with_ceiling(memory_ceiling: u64) -> Self {
        let mut recorder = Box::new(Recorder::default());
        let config = ak_runtime_config {
            struct_size: std::mem::size_of::<ak_runtime_config>() as u32,
            worker_threads: 2,
            memory_ceiling,
        };
        let mut runtime = AK_HANDLE_NONE;

        // SAFETY: the config and the out pointer are live for the call, and the recorder outlives
        // the runtime because this struct owns both and drops them in that order.
        let status = unsafe {
            ak_runtime_create(
                &config,
                Some(support::on_event),
                recorder.as_mut() as *mut Recorder as *mut c_void,
                &mut runtime,
            )
        };
        assert_eq!(status, ak_status::AK_STATUS_OK);
        assert_eq!(
            ak_runtime_status(runtime),
            ak_runtime_state::AK_RUNTIME_RUNNING
        );

        Self { runtime, recorder }
    }

    fn channel(&self, endpoint: &str) -> ak_handle {
        let json = format!(r#"{{"endpoint":"{endpoint}"}}"#);
        let mut channel = AK_HANDLE_NONE;
        // SAFETY: the JSON and the out pointer are live for the call.
        let status = unsafe {
            ak_channel_create(
                self.runtime,
                ak_bytes_in {
                    ptr: json.as_ptr(),
                    len: json.len(),
                },
                &mut channel,
            )
        };
        assert_eq!(status, ak_status::AK_STATUS_OK, "{json}");
        channel
    }

    /// Shuts the runtime down and waits for it to say nothing of it is outstanding.
    fn stop(&self) {
        assert_eq!(
            ak_runtime_begin_shutdown(self.runtime),
            ak_status::AK_STATUS_OK
        );
        self.await_state(ak_runtime_state::AK_RUNTIME_QUIESCENT);
    }

    fn await_state(&self, wanted: ak_runtime_state) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if ak_runtime_status(self.runtime) == wanted {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!(
            "the runtime is {:?} and not {wanted:?}; it still holds {} events",
            ak_runtime_status(self.runtime),
            self.recorder.len()
        );
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        if ak_runtime_status(self.runtime) != ak_runtime_state::AK_RUNTIME_QUIESCENT {
            self.stop();
        }
        assert_eq!(ak_runtime_destroy(self.runtime), ak_status::AK_STATUS_OK);
    }
}

/// One unary call: lend a buffer, fill it, commit, half-close.
fn send_one(call: ak_handle, message: &[u8]) {
    let mut buffer = ak_buffer {
        ptr: std::ptr::null_mut(),
        len: 0,
        owner: std::ptr::null_mut(),
    };
    // SAFETY: the out pointer is live for the call.
    let status = unsafe { ak_get_call_buffer(call, message.len(), &mut buffer) };
    assert_eq!(status, ak_status::AK_STATUS_OK);
    assert_eq!(buffer.len, message.len());

    // SAFETY: the library lent exactly `message.len()` writable bytes at `ptr`.
    unsafe { std::ptr::copy_nonoverlapping(message.as_ptr(), buffer.ptr, message.len()) };

    // SAFETY: the buffer is the one just lent and has not been given back.
    assert_eq!(
        unsafe { ak_call_send_message(call, buffer) },
        ak_status::AK_STATUS_OK
    );
    assert_eq!(ak_call_end_send(call), ak_status::AK_STATUS_OK);
}

fn start_call(channel: ak_handle, method: &str, metadata: &[u8]) -> ak_handle {
    let options = ak_call_start_options {
        struct_size: std::mem::size_of::<ak_call_start_options>() as u32,
        method: ak_bytes_in {
            ptr: method.as_ptr(),
            len: method.len(),
        },
        metadata: ak_bytes_in {
            ptr: metadata.as_ptr(),
            len: metadata.len(),
        },
    };
    let mut call = AK_HANDLE_NONE;
    // SAFETY: the options, their views and the out pointer are live for the call.
    let status = unsafe { ak_call_start(channel, &options, std::ptr::null_mut(), &mut call) };
    assert_eq!(status, ak_status::AK_STATUS_OK);
    call
}

// ---------------------------------------------------------------- the deliverable

#[test]
fn a_unary_call_through_the_abi_reaches_a_grpc_server_and_comes_back() {
    let server = TestServer::start();
    let host = Host::start();
    let channel = host.channel(&server.endpoint);

    let call = start_call(channel, ECHO, &blob(&[(b"x-request", b"ping")]));
    send_one(call, b"hello");

    let seen = host.recorder.await_terminal();

    // The order the ABI promises of the data events: the metadata first, then the messages, then
    // the terminal. WRITE_DONE is a second domain and may land anywhere among them, so it is
    // asserted as present and in order against itself rather than pinned to a position.
    assert_eq!(
        seen.data_kinds(),
        vec![
            ak_event_kind::AK_EVENT_INITIAL_METADATA,
            ak_event_kind::AK_EVENT_MESSAGE,
            ak_event_kind::AK_EVENT_STATUS,
        ],
        "{seen:?}"
    );
    assert_eq!(
        seen.kinds()
            .iter()
            .filter(|kind| **kind == ak_event_kind::AK_EVENT_WRITE_DONE)
            .count(),
        1,
        "one acquittal for the one accepted send"
    );
    assert_eq!(seen.message_payloads(), vec![b"hello".to_vec()]);
    assert_eq!(seen.status_code(), Some(0));
    assert_eq!(
        seen.initial_metadata().get(&b"x-echoed"[..]),
        Some(&b"ping".to_vec()),
        "the request metadata crossed the ABI and its answer came back"
    );

    // Nothing of the call is outstanding, so the runtime has taken its handle back on its own.
    host.recorder.await_call_reclaimed(call);
    assert_eq!(
        ak_call_cancel(call),
        ak_status::AK_STATUS_HANDLE_STALE,
        "a reclaimed call names nothing"
    );

    ak_channel_release(channel);
    host.stop();
}

#[test]
fn a_refused_method_comes_back_as_its_status_behind_a_synthesized_metadata_event() {
    let server = TestServer::start();
    let host = Host::start();
    let channel = host.channel(&server.endpoint);

    let call = start_call(channel, FAIL, &[]);
    send_one(call, b"x");

    let seen = host.recorder.await_terminal();

    // A Trailers-Only refusal carries no response head on the wire; the ABI still emits exactly
    // one INITIAL_METADATA, first, and it takes a delivery credit like any other.
    assert_eq!(
        seen.data_kinds()[0],
        ak_event_kind::AK_EVENT_INITIAL_METADATA
    );
    assert!(seen.initial_metadata().is_empty());
    assert!(seen.first_data_event_was_owned(), "empty is not unowned");
    assert_eq!(seen.status_code(), Some(7), "PERMISSION_DENIED");
    assert_eq!(seen.status_message(), "not for you");

    ak_channel_release(channel);
    host.stop();
}

#[test]
fn the_send_window_refuses_a_second_buffer_until_a_write_is_acquitted() {
    let server = TestServer::start();
    let host = Host::start();
    let channel = host.channel(&server.endpoint);
    let call = start_call(channel, ECHO, &[]);

    let mut first = ak_buffer {
        ptr: std::ptr::null_mut(),
        len: 0,
        owner: std::ptr::null_mut(),
    };
    // SAFETY: the out pointer is live for the call.
    assert_eq!(
        unsafe { ak_get_call_buffer(call, 4, &mut first) },
        ak_status::AK_STATUS_OK
    );

    // A window of one, and one buffer out: backpressure, not an error.
    let mut second = first;
    // SAFETY: as above.
    assert_eq!(
        unsafe { ak_get_call_buffer(call, 4, &mut second) },
        ak_status::AK_STATUS_SLOT_BUSY
    );

    // Giving the buffer back unused frees its slot, which is the other exit the ABI names.
    // SAFETY: `first` is the buffer just lent and has not been given back.
    unsafe { ak_return_call_buffer(first) };
    // SAFETY: the out pointer is live for the call.
    assert_eq!(
        unsafe { ak_get_call_buffer(call, 4, &mut second) },
        ak_status::AK_STATUS_OK
    );
    // SAFETY: `second` is the buffer just lent.
    unsafe { ak_return_call_buffer(second) };

    assert_eq!(ak_call_cancel(call), ak_status::AK_STATUS_OK);
    host.recorder.await_terminal();
    ak_channel_release(channel);
    host.stop();
}

#[test]
fn the_ceiling_refuses_what_will_never_fit_apart_from_what_does_not_fit_yet() {
    let server = TestServer::start();
    let host = Host::with_ceiling(64);
    let channel = host.channel(&server.endpoint);
    let call = start_call(channel, ECHO, &[]);

    let mut buffer = ak_buffer {
        ptr: std::ptr::null_mut(),
        len: 0,
        owner: std::ptr::null_mut(),
    };

    // Past the ceiling itself: no return by anyone will ever make room, so the refusal is
    // permanent and the host is told not to retry.
    // SAFETY: the out pointer is live for the call.
    assert_eq!(
        unsafe { ak_get_call_buffer(call, 65, &mut buffer) },
        ak_status::AK_STATUS_MESSAGE_TOO_LARGE
    );

    // Within the ceiling, so it is lent and it is what the runtime reports as occupied.
    // SAFETY: as above.
    assert_eq!(
        unsafe { ak_get_call_buffer(call, 40, &mut buffer) },
        ak_status::AK_STATUS_OK
    );
    let mut usage = ak_memory_usage::default();
    // SAFETY: the out pointer is live for the call.
    assert_eq!(
        unsafe { ak_runtime_memory_usage(host.runtime, &mut usage) },
        ak_status::AK_STATUS_OK
    );
    assert_eq!(
        usage,
        ak_memory_usage {
            bytes_used: 40,
            ceiling: 64
        }
    );

    // Giving it back is what frees the bytes, which only a fall in the total proves.
    // SAFETY: `buffer` is the one just lent and has not been given back.
    unsafe { ak_return_call_buffer(buffer) };
    // SAFETY: the out pointer is live for the call.
    unsafe { ak_runtime_memory_usage(host.runtime, &mut usage) };
    assert_eq!(usage.bytes_used, 0);

    assert_eq!(ak_call_cancel(call), ak_status::AK_STATUS_OK);
    host.recorder.await_terminal();
    ak_channel_release(channel);
    host.stop();
}

#[test]
fn a_runtime_the_host_still_owes_says_so_and_reaches_quiescence_when_it_is_paid() {
    let server = TestServer::start();
    let host = Host::start();
    // This host deliberately does not consume, which is what gives the runtime something to
    // report; with one delivery credit it therefore sees the metadata and no message after it.
    host.recorder.hold_payloads();
    let channel = host.channel(&server.endpoint);

    let call = start_call(channel, ECHO, &[]);
    send_one(call, b"held");
    host.recorder.await_metadata();

    // The payloads are deliberately not consumed yet.
    ak_channel_release(channel);
    assert_eq!(
        ak_runtime_begin_shutdown(host.runtime),
        ak_status::AK_STATUS_OK
    );

    host.recorder.await_shutdown();
    assert_eq!(
        host.recorder.shutdown_debt(),
        Some(ak_host_debt::AK_HOST_MUST_RETURN),
        "the host is still holding the call's payloads"
    );
    assert_eq!(
        ak_runtime_status(host.runtime),
        ak_runtime_state::AK_RUNTIME_GRPC_STOPPED,
        "stopped is not quiescent while the host holds something"
    );
    assert_eq!(
        ak_runtime_destroy(host.runtime),
        ak_status::AK_STATUS_INVALID_STATE,
        "destroying before quiescence is refused"
    );

    host.recorder.consume_all();
    host.await_state(ak_runtime_state::AK_RUNTIME_QUIESCENT);
    assert!(
        host.recorder
            .kinds()
            .contains(&ak_event_kind::AK_EVENT_RESOURCES_RELEASED),
        "the second event is owed and arrives"
    );
}

#[test]
fn a_runtime_that_owes_nothing_gets_one_shutdown_event_and_no_second() {
    let host = Host::start();
    host.stop();

    let kinds = host.recorder.kinds();
    assert_eq!(kinds, vec![ak_event_kind::AK_EVENT_SHUTDOWN_COMPLETE]);
    assert_eq!(
        host.recorder.shutdown_debt(),
        Some(ak_host_debt::AK_HOST_NOTHING_TO_RETURN)
    );
}

// ---------------------------------------------------------------- what the ABI refuses

#[test]
fn a_struct_of_an_unknown_size_is_refused_rather_than_read() {
    let host = Host::start();
    let channel = host.channel("http://127.0.0.1:1");

    let options = ak_call_start_options {
        // A caller compiled against a version this library does not know.
        struct_size: 7,
        method: ak_bytes_in {
            ptr: ECHO.as_ptr(),
            len: ECHO.len(),
        },
        metadata: ak_bytes_in {
            ptr: std::ptr::null(),
            len: 0,
        },
    };
    let mut call = AK_HANDLE_NONE;
    // SAFETY: the options and the out pointer are live for the call.
    let status = unsafe { ak_call_start(channel, &options, std::ptr::null_mut(), &mut call) };

    assert_eq!(status, ak_status::AK_STATUS_INVALID_ARG);
    assert_eq!(call, AK_HANDLE_NONE, "nothing was started");
    ak_channel_release(channel);
}

#[test]
fn a_token_naming_nothing_is_refused_rather_than_dereferenced() {
    assert_eq!(
        ak_call_cancel(AK_HANDLE_NONE),
        ak_status::AK_STATUS_HANDLE_STALE
    );
    assert_eq!(
        ak_call_end_send(u64::MAX),
        ak_status::AK_STATUS_HANDLE_STALE
    );
    assert_eq!(
        ak_runtime_begin_shutdown(u64::MAX),
        ak_status::AK_STATUS_HANDLE_STALE
    );
    // Freeing something that names nothing is a no-op rather than a fault.
    ak_channel_release(u64::MAX);
}

#[test]
fn no_call_starts_on_a_runtime_that_is_stopping() {
    let server = TestServer::start();
    let host = Host::start();
    let channel = host.channel(&server.endpoint);

    host.stop();

    let options = ak_call_start_options {
        struct_size: std::mem::size_of::<ak_call_start_options>() as u32,
        method: ak_bytes_in {
            ptr: ECHO.as_ptr(),
            len: ECHO.len(),
        },
        metadata: ak_bytes_in {
            ptr: std::ptr::null(),
            len: 0,
        },
    };
    let mut call = AK_HANDLE_NONE;
    // SAFETY: the options and the out pointer are live for the call.
    let status = unsafe { ak_call_start(channel, &options, std::ptr::null_mut(), &mut call) };

    // The channel went with the shutdown, so its token names nothing.
    assert_eq!(status, ak_status::AK_STATUS_HANDLE_STALE);
}

#[test]
fn the_abi_version_is_the_one_the_header_carries() {
    assert_eq!(ak_abi_version(), AK_ABI_VERSION);
}

#[test]
fn consuming_an_unowned_payload_is_a_no_op() {
    // SAFETY: the empty, unowned value the ABI says an event with no payload carries.
    unsafe {
        ak_event_consumed(ak_bytes {
            ptr: std::ptr::null(),
            len: 0,
            owner: std::ptr::null_mut(),
        })
    };
}

// Keeps `Seen` in the crate's use even when only some tests are run.
fn _assert_seen_is_debug(seen: &Seen) -> String {
    format!("{seen:?}")
}
