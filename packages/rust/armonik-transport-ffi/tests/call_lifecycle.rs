//! Deadlines, cancellation, backpressure, misuse, and leaks.
//!
//! The error branches, in other words — which is where an FFI actually fails. A crate that only ever
//! ran its happy paths would be free to leak a handle on every cancelled call, hang forever on a
//! request stream the caller forgot to close, or dereference a pointer that was freed a moment ago,
//! and every test in `calls.rs` would still pass.
//!
//! Each test here names the property it protects rather than the code path it walks, so a failure
//! says what broke for the caller.

mod common;

use std::time::{Duration, Instant};

use armonik_transport_ffi::status;
use bytes::Bytes;
use common::abi::{Client, Kind, Poll, StartOptions};
use common::server::{serve, TestService, METHOD_PATH};

/// gRPC's `CANCELLED` and `DEADLINE_EXCEEDED`.
const GRPC_CANCELLED: i32 = 1;
const GRPC_DEADLINE_EXCEEDED: i32 = 4;

/// Long enough that a slow CI runner does not trip it, short enough not to pad the suite.
const SHORT_DEADLINE_MS: i64 = 300;

// --- Deadlines ---------------------------------------------------------------------------------

#[test]
fn a_deadline_expires_while_the_server_never_answers() {
    let endpoint = serve(TestService::hang());

    let client = Client::to(&endpoint, &[]);
    let call = client.start(
        METHOD_PATH,
        Kind::Unary,
        StartOptions::deadline(SHORT_DEADLINE_MS),
    );
    call.send_ok(b"ping");
    call.close_send_ok();

    let (messages, outcome) = call.drain();

    assert!(messages.is_empty());
    assert_eq!(outcome.code, GRPC_DEADLINE_EXCEEDED);
}

#[test]
fn a_deadline_expires_while_waiting_for_the_request_messages() {
    // The caller never closes its send side, so a retryable call sits in the phase that buffers the
    // request message. That wait used to be covered by neither the deadline nor the cancellation
    // signal, which parked the driving task for the life of the process: a leak of a task, a
    // connection and an OS handle per call, triggered by nothing worse than a forgotten `Dispose`.
    let endpoint = serve(TestService::canned([Bytes::from_static(b"never sent")]));

    let client = Client::to(&endpoint, &[]);
    let call = client.start(
        METHOD_PATH,
        Kind::Unary,
        StartOptions::deadline(SHORT_DEADLINE_MS),
    );
    call.send_ok(b"ping");
    // No `close_send`, deliberately.

    let (messages, outcome) = call.drain();

    assert!(messages.is_empty());
    assert_eq!(outcome.code, GRPC_DEADLINE_EXCEEDED);
}

#[test]
fn one_deadline_covers_every_retry_rather_than_restarting_per_attempt() {
    // Three attempts against a server that never answers, under a deadline shorter than three times
    // itself. If the deadline were re-anchored per attempt — the obvious way to write it, and wrong —
    // the call would outlive its deadline roughly threefold.
    let endpoint = serve(TestService::hang());

    let client = Client::to(&endpoint, &Client::quick_retry());
    let call = client.start(
        METHOD_PATH,
        Kind::Unary,
        StartOptions::deadline(SHORT_DEADLINE_MS),
    );
    call.send_ok(b"ping");
    call.close_send_ok();

    let started = Instant::now();
    let (_, outcome) = call.drain();
    let elapsed = started.elapsed();

    assert_eq!(outcome.code, GRPC_DEADLINE_EXCEEDED);
    // Generously bounded: the assertion is "one budget, not one per attempt", not a timing promise.
    assert!(
        elapsed < Duration::from_millis(SHORT_DEADLINE_MS as u64 * 2),
        "the call took {elapsed:?}, which suggests the deadline restarted per attempt"
    );
}

#[test]
fn a_deadline_that_has_not_expired_does_not_interfere() {
    // The mirror of the tests above: a deadline must bound a call, not shorten it.
    let endpoint = serve(TestService::canned([Bytes::from_static(b"pong")]));

    let client = Client::to(&endpoint, &[]);
    let call = client.start(METHOD_PATH, Kind::Unary, StartOptions::deadline(60_000));
    call.send_ok(b"ping");
    call.close_send_ok();

    let (messages, outcome) = call.drain();

    assert_eq!(messages, vec![b"pong".to_vec()]);
    assert_eq!(outcome.code, 0);
}

// --- Cancellation ------------------------------------------------------------------------------

#[test]
fn cancelling_a_call_the_server_never_answers_reports_cancelled() {
    let endpoint = serve(TestService::hang());

    let client = Client::to(&endpoint, &[]);
    let call = client.start(METHOD_PATH, Kind::Unary, StartOptions::default());
    call.send_ok(b"ping");
    call.close_send_ok();

    assert_eq!(call.cancel(), status::OK);

    let (messages, outcome) = call.drain();
    assert!(messages.is_empty());
    assert_eq!(outcome.code, GRPC_CANCELLED);
}

#[test]
fn cancelling_while_waiting_for_the_request_messages_reports_cancelled() {
    // The other half of the buffering-phase gap: before, `ak_call_cancel` had no effect at all until
    // a request message happened to arrive, so cancelling a call the caller had not finished writing
    // silently did nothing.
    let endpoint = serve(TestService::canned([Bytes::from_static(b"never sent")]));

    let client = Client::to(&endpoint, &[]);
    let call = client.start(METHOD_PATH, Kind::Unary, StartOptions::default());
    call.send_ok(b"ping");
    // No `close_send`: the driving task is parked waiting for it.

    assert_eq!(call.cancel(), status::OK);

    let (_, outcome) = call.drain();
    assert_eq!(outcome.code, GRPC_CANCELLED);
}

#[test]
fn cancelling_mid_stream_stops_a_call_that_would_otherwise_continue() {
    let service = TestService::echo_each(Bytes::from_static(b"-ack"));
    let endpoint = serve(service);

    let client = Client::to(&endpoint, &[]);
    let call = client.start(METHOD_PATH, Kind::BidiStreaming, StartOptions::default());
    call.send_ok(b"one");
    call.wait_until("the first echo", |call| {
        matches!(call.try_recv(), Poll::Message(_))
    });

    // The send side is still open and the server is still willing to answer: only the cancellation
    // ends this.
    assert_eq!(call.cancel(), status::OK);

    let (_, outcome) = call.drain();
    assert_eq!(outcome.code, GRPC_CANCELLED);
}

#[test]
fn cancelling_is_idempotent_and_safe_after_completion() {
    let endpoint = serve(TestService::canned([Bytes::from_static(b"pong")]));

    let client = Client::to(&endpoint, &[]);
    let call = client.start(METHOD_PATH, Kind::Unary, StartOptions::default());
    call.send_ok(b"ping");
    call.close_send_ok();
    let (_, outcome) = call.drain();
    assert_eq!(outcome.code, 0);

    // .NET's `CancellationToken` may fire at any point, including after the call it was meant to
    // cancel already returned.
    assert_eq!(call.cancel(), status::OK);
    assert_eq!(call.cancel(), status::OK);
    assert_eq!(
        call.status().expect("still readable").code,
        0,
        "a late cancellation must not rewrite an outcome the caller already has"
    );
}

// --- Backpressure ------------------------------------------------------------------------------

#[test]
fn a_full_send_queue_reports_would_block_and_recovers_once_it_drains() {
    // The server neither reads the request stream nor answers, so the HTTP/2 flow-control window
    // closes, `tonic` stops pulling, and the ABI's send queue fills. That is the only deterministic
    // way to reach `AK_WOULD_BLOCK`: a queue that merely happened to be full would make this test a
    // coin toss.
    let mut service = TestService::hang_without_reading();
    let gate = service.gated();
    let endpoint = serve(service);

    let client = Client::to(&endpoint, &[]);
    let call = client.start(
        METHOD_PATH,
        Kind::ClientStreaming,
        StartOptions::capacity(1),
    );

    // 16 KiB a time: large enough to exhaust a default 64 KiB window in a handful of messages,
    // rather than needing thousands.
    let payload = vec![0x7e; 16 * 1024];
    let mut accepted = 0;
    let blocked_at = loop {
        match call.send(&payload) {
            status::OK => {
                accepted += 1;
                assert!(
                    accepted < 1_000,
                    "the send queue never filled; backpressure is not being applied"
                );
            }
            status::WOULD_BLOCK => break accepted,
            other => panic!("unexpected send status {other}"),
        }
    };
    assert!(
        blocked_at > 0,
        "at least one message should have been accepted before the queue filled"
    );

    // Letting the handler run drains the stream, which reopens the window and frees slots. The
    // recovery is what makes `WOULD_BLOCK` a retryable answer rather than a dead end.
    gate.open();
    call.wait_until("a send slot to free up", |call| {
        call.send(&payload) == status::OK
    });
}

#[test]
fn a_full_receive_queue_does_not_lose_messages() {
    // The receive queue is bounded too. A caller that stops draining must see the stream pause and
    // then resume exactly where it left off — never skip a message.
    const MESSAGES: usize = 20;

    let service =
        TestService::canned((0..MESSAGES).map(|index| Bytes::from(format!("m{index:02}"))));
    let endpoint = serve(service);

    let client = Client::to(&endpoint, &[]);
    let call = client.start(
        METHOD_PATH,
        Kind::ServerStreaming,
        StartOptions::capacity(1),
    );
    call.send_ok(b"start");
    call.close_send_ok();

    // Deliberately slow: pause between reads so the queue is full most of the time.
    let mut received: Vec<String> = Vec::new();
    let outcome = loop {
        match call.try_recv() {
            Poll::Message(message) => {
                received.push(String::from_utf8(message).expect("utf-8"));
                std::thread::sleep(Duration::from_millis(1));
            }
            Poll::Pending => std::thread::sleep(Duration::from_millis(1)),
            Poll::Completed => break call.status().expect("a status"),
        }
    };

    assert_eq!(outcome.code, 0);
    let expected: Vec<String> = (0..MESSAGES).map(|index| format!("m{index:02}")).collect();
    assert_eq!(received, expected, "no message may be dropped or reordered");
}

// --- Misuse ------------------------------------------------------------------------------------

#[test]
fn sending_after_the_send_side_is_closed_is_rejected() {
    let endpoint = serve(TestService::canned([Bytes::from_static(b"pong")]));

    let client = Client::to(&endpoint, &[]);
    let call = client.start(METHOD_PATH, Kind::Unary, StartOptions::default());
    call.send_ok(b"ping");
    call.close_send_ok();

    assert_eq!(call.send(b"too late"), status::INVALID_STATE);
}

#[test]
fn closing_the_send_side_twice_is_rejected() {
    let endpoint = serve(TestService::canned([Bytes::from_static(b"pong")]));

    let client = Client::to(&endpoint, &[]);
    let call = client.start(METHOD_PATH, Kind::Unary, StartOptions::default());
    call.close_send_ok();

    assert_eq!(
        call.close_send(),
        status::INVALID_STATE,
        "the second close has nothing left to close, and must say so rather than pretend"
    );
}

#[test]
fn a_status_read_before_completion_is_rejected_rather_than_invented() {
    let endpoint = serve(TestService::hang());

    let client = Client::to(&endpoint, &[]);
    let call = client.start(METHOD_PATH, Kind::Unary, StartOptions::default());
    call.send_ok(b"ping");
    call.close_send_ok();

    assert!(
        call.status().is_none(),
        "a call still in flight has no outcome to report"
    );
}

#[test]
fn an_unknown_method_kind_is_rejected() {
    let endpoint = serve(TestService::canned([Bytes::from_static(b"pong")]));
    let client = Client::to(&endpoint, &[]);

    let mut out: *mut armonik_transport_ffi::ak_call = std::ptr::null_mut();
    // SAFETY: a live client; `4` is deliberately outside the documented range, which is the
    // assertion; both out-parameters point at live locals.
    let started = unsafe {
        armonik_transport_ffi::ak_call_start(
            client.as_ptr(),
            METHOD_PATH.as_ptr(),
            METHOD_PATH.len(),
            4,
            std::ptr::null(),
            0,
            0,
            8,
            std::ptr::addr_of_mut!(out),
            std::ptr::null_mut(),
        )
    };

    assert_eq!(started, status::INVALID_STATE);
    assert!(out.is_null());
}

#[test]
fn a_method_path_that_is_not_a_path_is_rejected_locally() {
    let endpoint = serve(TestService::canned([Bytes::from_static(b"pong")]));
    let client = Client::to(&endpoint, &[]);

    for path in ["not a path", ""] {
        let (code, message) = client
            .try_start(path, Kind::Unary, StartOptions::default())
            .err()
            .unwrap_or_else(|| panic!("{path:?} should have been rejected"));
        assert_eq!(code, status::INVALID_STATE);
        assert!(
            !message.is_empty(),
            "a rejection should explain itself: {path:?}"
        );
    }
}

#[test]
fn a_malformed_metadata_blob_is_rejected_before_the_call_starts() {
    let endpoint = serve(TestService::canned([Bytes::from_static(b"pong")]));
    let client = Client::to(&endpoint, &[]);

    // A count claiming one entry, with nothing after it.
    let truncated = 1u32.to_ne_bytes().to_vec();
    let (code, _) = client
        .try_start(
            METHOD_PATH,
            Kind::Unary,
            StartOptions {
                metadata: truncated,
                ..StartOptions::default()
            },
        )
        .err()
        .expect("a truncated blob should be rejected");

    assert_eq!(code, status::INVALID_STATE);
}

#[test]
fn every_call_entry_point_rejects_a_freed_handle() {
    // A use-after-free must be reported, not dereferenced. Each of these would otherwise read
    // through a dangling pointer — the failure mode that turns a .NET double-`Dispose` into memory
    // corruption in a customer's process.
    let endpoint = serve(TestService::canned([Bytes::from_static(b"pong")]));
    let client = Client::to(&endpoint, &[]);
    let call = client.start(METHOD_PATH, Kind::Unary, StartOptions::default());
    let dangling = call.free_early();

    let mut code = 0i32;
    let mut state = 0i32;
    let mut bytes = [armonik_transport_ffi::ak_bytes {
        ptr: std::ptr::null(),
        len: 0,
        owner: std::ptr::null_mut(),
    }; 3];

    // SAFETY: every call below deliberately passes a freed handle. The ABI documents each as
    // returning `AK_INVALID_HANDLE` without touching the memory, which is exactly what is asserted;
    // if any of them did dereference it, this test would corrupt memory rather than fail.
    unsafe {
        assert_eq!(
            armonik_transport_ffi::ak_call_try_send(dangling, b"x".as_ptr(), 1),
            status::INVALID_HANDLE
        );
        assert_eq!(
            armonik_transport_ffi::ak_call_close_send(dangling),
            status::INVALID_HANDLE
        );
        assert_eq!(
            armonik_transport_ffi::ak_call_try_recv(
                dangling,
                std::ptr::addr_of_mut!(bytes[0]),
                std::ptr::addr_of_mut!(state)
            ),
            status::INVALID_HANDLE
        );
        assert_eq!(
            armonik_transport_ffi::ak_call_try_headers(dangling, std::ptr::addr_of_mut!(bytes[1])),
            status::INVALID_HANDLE
        );
        assert_eq!(
            armonik_transport_ffi::ak_call_status(
                dangling,
                std::ptr::addr_of_mut!(code),
                std::ptr::addr_of_mut!(bytes[1]),
                std::ptr::addr_of_mut!(bytes[2])
            ),
            status::INVALID_HANDLE
        );
        assert_eq!(
            armonik_transport_ffi::ak_call_cancel(dangling),
            status::INVALID_HANDLE
        );
        assert!(
            armonik_transport_ffi::ak_call_wait_handle(dangling).is_null(),
            "a freed call has no handle to lend"
        );
        // And freeing it a second time is a no-op rather than a double free.
        armonik_transport_ffi::ak_call_free(dangling);
    }
}

#[test]
fn every_call_entry_point_rejects_null() {
    let mut state = 0i32;
    let mut bytes = armonik_transport_ffi::ak_bytes {
        ptr: std::ptr::null(),
        len: 0,
        owner: std::ptr::null_mut(),
    };

    // SAFETY: null is explicitly allowed everywhere below, and reported rather than dereferenced.
    unsafe {
        assert_eq!(
            armonik_transport_ffi::ak_call_try_send(std::ptr::null(), std::ptr::null(), 0),
            status::NULL_ARGUMENT
        );
        assert_eq!(
            armonik_transport_ffi::ak_call_close_send(std::ptr::null()),
            status::NULL_ARGUMENT
        );
        assert_eq!(
            armonik_transport_ffi::ak_call_try_recv(
                std::ptr::null(),
                std::ptr::addr_of_mut!(bytes),
                std::ptr::addr_of_mut!(state)
            ),
            status::NULL_ARGUMENT
        );
        assert_eq!(
            armonik_transport_ffi::ak_call_cancel(std::ptr::null()),
            status::NULL_ARGUMENT
        );
        // A null out-parameter is rejected too, not just a null handle.
        assert_eq!(
            armonik_transport_ffi::ak_call_try_headers(std::ptr::null(), std::ptr::null_mut()),
            status::NULL_ARGUMENT
        );
        assert_eq!(
            armonik_transport_ffi::ak_call_status(
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut()
            ),
            status::NULL_ARGUMENT
        );
        assert!(armonik_transport_ffi::ak_call_wait_handle(std::ptr::null()).is_null());
        // Freeing null is documented as a no-op, not an error.
        armonik_transport_ffi::ak_call_free(std::ptr::null_mut());
    }
}

#[test]
fn a_call_freed_mid_flight_is_safe() {
    // The .NET side frees on `Dispose`, which a `using` block runs on the way out of an exception —
    // i.e. routinely, while the call is still running.
    let endpoint = serve(TestService::hang());
    let client = Client::to(&endpoint, &[]);

    for _ in 0..16 {
        let call = client.start(METHOD_PATH, Kind::Unary, StartOptions::default());
        call.send_ok(b"ping");
        // Dropped here, in every state from "just started" to "waiting on the server".
    }
}
