//! Every call shape, driven through the C ABI against a real gRPC server.
//!
//! These are the tests that say the ABI works. Everything below goes through the exact entry points
//! .NET will call, in the same order, over a real HTTP/2 connection to a real `tonic` server — no
//! mocking of the transport, no calling into private helpers. The misuse, deadline, cancellation and
//! backpressure paths live in `call_lifecycle.rs`.
//!
//! Retry expectations are asserted against what the *server* saw, not just the status the client
//! ended up with: a policy that silently never fires would otherwise pass every happy-path test.

mod common;

use armonik_transport::reexports::tonic::Code;
use armonik_transport_ffi::status;
use bytes::Bytes;
use common::abi::{Client, Kind, Poll, StartOptions};
use common::server::{serve, TestService, METHOD_PATH};

/// The gRPC `OK` code, as `ak_call_status` reports it.
const GRPC_OK: i32 = 0;
const GRPC_UNAVAILABLE: i32 = 14;
const GRPC_INVALID_ARGUMENT: i32 = 3;
const GRPC_UNIMPLEMENTED: i32 = 12;

#[test]
fn a_unary_call_sends_one_message_and_receives_one() {
    let service = TestService::canned([Bytes::from_static(b"pong")]);
    let endpoint = serve(service.clone());

    let client = Client::to(&endpoint, &[]);
    let call = client.start(METHOD_PATH, Kind::Unary, StartOptions::default());
    call.send_ok(b"ping");
    call.close_send_ok();

    let (messages, outcome) = call.drain();

    assert_eq!(messages, vec![b"pong".to_vec()]);
    assert_eq!(outcome.code, GRPC_OK);
    assert_eq!(outcome.message, "");
    assert_eq!(
        service.messages_received(),
        vec![vec![Bytes::from_static(b"ping")]],
        "the server should have seen exactly the one request message"
    );
}

#[test]
fn a_server_streaming_call_receives_every_message_in_order() {
    let service = TestService::canned([
        Bytes::from_static(b"one"),
        Bytes::from_static(b"two"),
        Bytes::from_static(b"three"),
    ]);
    let endpoint = serve(service.clone());

    let client = Client::to(&endpoint, &[]);
    let call = client.start(METHOD_PATH, Kind::ServerStreaming, StartOptions::default());
    call.send_ok(b"start");
    call.close_send_ok();

    let (messages, outcome) = call.drain();

    assert_eq!(
        messages,
        vec![b"one".to_vec(), b"two".to_vec(), b"three".to_vec()],
        "order matters: a queue that reordered messages would corrupt every stream"
    );
    assert_eq!(outcome.code, GRPC_OK);
}

#[test]
fn a_client_streaming_call_sends_every_message_and_receives_one() {
    let service = TestService::canned([Bytes::from_static(b"summary")]);
    let endpoint = serve(service.clone());

    let client = Client::to(&endpoint, &[]);
    let call = client.start(METHOD_PATH, Kind::ClientStreaming, StartOptions::default());
    for payload in [b"first", b"secnd", b"third"] {
        call.send_ok(payload);
    }
    call.close_send_ok();

    let (messages, outcome) = call.drain();

    assert_eq!(messages, vec![b"summary".to_vec()]);
    assert_eq!(outcome.code, GRPC_OK);
    assert_eq!(
        service.messages_received(),
        vec![vec![
            Bytes::from_static(b"first"),
            Bytes::from_static(b"secnd"),
            Bytes::from_static(b"third"),
        ]],
        "every message sent before `close_send` must reach the server, in order"
    );
}

#[test]
fn a_bidirectional_call_interleaves_sends_and_receives() {
    // The echoing service replies to each message as it arrives rather than after the request stream
    // ends, so this really does interleave: the assertion below fails if the ABI cannot receive while
    // its send side is still open.
    let service = TestService::echo_each(Bytes::from_static(b"-ack"));
    let endpoint = serve(service.clone());

    let client = Client::to(&endpoint, &[]);
    let call = client.start(METHOD_PATH, Kind::BidiStreaming, StartOptions::default());

    let mut received = Vec::new();
    for payload in [b"one".as_slice(), b"two".as_slice(), b"six".as_slice()] {
        call.send_ok(payload);
        // Each reply is read back before the next message is sent, which is only possible if the two
        // directions are independent.
        call.wait_until("the echo of the message just sent", |call| {
            match call.try_recv() {
                Poll::Message(message) => {
                    received.push(message);
                    true
                }
                Poll::Pending => false,
                Poll::Completed => panic!("the call ended before it echoed everything"),
            }
        });
    }
    call.close_send_ok();

    let (trailing, outcome) = call.drain();
    received.extend(trailing);

    assert_eq!(
        received,
        vec![
            b"one-ack".to_vec(),
            b"two-ack".to_vec(),
            b"six-ack".to_vec()
        ]
    );
    assert_eq!(outcome.code, GRPC_OK);
}

#[test]
fn a_call_with_no_request_message_still_completes() {
    // Not a shape any ArmoniK RPC uses, but the ABI has to be honest about it: closing the send side
    // without sending anything is a well-formed client stream of length zero, not an error.
    let service = TestService::canned([Bytes::from_static(b"nothing to do")]);
    let endpoint = serve(service.clone());

    let client = Client::to(&endpoint, &[]);
    let call = client.start(METHOD_PATH, Kind::ClientStreaming, StartOptions::default());
    call.close_send_ok();

    let (messages, outcome) = call.drain();

    assert_eq!(messages, vec![b"nothing to do".to_vec()]);
    assert_eq!(outcome.code, GRPC_OK);
    assert_eq!(service.messages_received(), vec![Vec::<Bytes>::new()]);
}

#[test]
fn an_empty_message_is_carried_rather_than_dropped() {
    // A zero-length protobuf message is the wire form of a message whose every field is defaulted —
    // extremely common — so "empty" must not be confused with "absent" anywhere along the path.
    let service = TestService::canned([Bytes::new()]);
    let endpoint = serve(service.clone());

    let client = Client::to(&endpoint, &[]);
    let call = client.start(METHOD_PATH, Kind::Unary, StartOptions::default());
    call.send_ok(b"");
    call.close_send_ok();

    let (messages, outcome) = call.drain();

    assert_eq!(outcome.code, GRPC_OK);
    assert_eq!(
        messages,
        vec![Vec::<u8>::new()],
        "an empty response message must still be reported as a message"
    );
    assert_eq!(service.messages_received(), vec![vec![Bytes::new()]]);
}

#[test]
fn a_large_message_survives_the_round_trip() {
    // Larger than the default 16 KiB HTTP/2 frame and than one gRPC length-prefixed read, so this
    // covers the reassembly the codec does rather than a single-frame happy path.
    let payload = Bytes::from(vec![0x5a; 512 * 1024]);
    let service = TestService::echo_each(Bytes::new());
    let endpoint = serve(service.clone());

    let client = Client::to(&endpoint, &[]);
    let call = client.start(METHOD_PATH, Kind::Unary, StartOptions::default());
    call.send_ok(&payload);
    call.close_send_ok();

    let (messages, outcome) = call.drain();

    assert_eq!(outcome.code, GRPC_OK);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].len(), payload.len());
    assert_eq!(messages[0], payload.as_ref());
}

#[test]
fn request_metadata_reaches_the_server_including_binary_values() {
    let service = TestService::canned([Bytes::from_static(b"ok")]);
    let endpoint = serve(service.clone());

    let binary: &[u8] = &[0, 1, 0xff, 0xfe];
    let client = Client::to(&endpoint, &[]);
    let call = client.start(
        METHOD_PATH,
        Kind::Unary,
        StartOptions::metadata([
            ("authorization", b"Bearer token".as_slice()),
            ("x-trace-bin", binary),
        ]),
    );
    call.send_ok(b"ping");
    call.close_send_ok();

    let (_, outcome) = call.drain();
    assert_eq!(outcome.code, GRPC_OK);

    let seen = service.metadata_seen();
    let seen = seen.first().expect("the server should have seen one call");
    assert_eq!(
        seen.get("authorization")
            .expect("authorization")
            .to_str()
            .expect("ascii"),
        "Bearer token"
    );
    assert_eq!(
        seen.get_bin("x-trace-bin")
            .expect("x-trace-bin")
            .to_bytes()
            .expect("decodable"),
        binary,
        "a `-bin` value must arrive as the raw bytes, not as their base64 storage"
    );
}

#[test]
fn response_headers_become_available_before_the_call_completes() {
    // `ResponseHeadersAsync` on the .NET side resolves as soon as the headers arrive, well before the
    // last message of a stream, so this must not be readable only at the end.
    let service = TestService::canned([Bytes::from_static(b"one"), Bytes::from_static(b"two")])
        .with_response_header("x-served-by", "the-test-service");
    let endpoint = serve(service.clone());

    let client = Client::to(&endpoint, &[]);
    let call = client.start(METHOD_PATH, Kind::ServerStreaming, StartOptions::default());
    call.send_ok(b"start");
    call.close_send_ok();

    assert_eq!(call.headers(), None, "nothing has arrived yet");

    call.wait_until("the response headers", |call| call.headers().is_some());
    let headers = call.headers().expect("just observed as present");
    assert!(
        headers
            .iter()
            .any(|(key, value)| key == "x-served-by" && value == b"the-test-service"),
        "the response headers should carry what the server set: {headers:?}"
    );

    // Reading them does not consume them: the contract says so, because .NET may look twice.
    assert_eq!(call.headers(), Some(headers));

    let (messages, outcome) = call.drain();
    assert_eq!(messages.len(), 2);
    assert_eq!(outcome.code, GRPC_OK);
}

#[test]
fn an_error_status_carries_its_code_message_and_trailers() {
    let service = TestService::canned([Bytes::from_static(b"never sent")])
        .failing_first(1, Code::InvalidArgument)
        .with_failure_trailer("x-reason", "because");
    let endpoint = serve(service.clone());

    let client = Client::to(&endpoint, &[]);
    let call = client.start(METHOD_PATH, Kind::Unary, StartOptions::default());
    call.send_ok(b"ping");
    call.close_send_ok();

    let (messages, outcome) = call.drain();

    assert!(messages.is_empty());
    assert_eq!(outcome.code, GRPC_INVALID_ARGUMENT);
    assert_eq!(outcome.message, "injected failure");
    assert!(
        outcome
            .trailers
            .iter()
            .any(|(key, value)| key == "x-reason" && value == b"because"),
        "the failure trailers should reach the caller: {:?}",
        outcome.trailers
    );
}

#[test]
fn a_status_can_be_read_more_than_once() {
    // .NET reads the status from more than one place — the `RpcException` it throws and
    // `GetStatus()` on the call — so, unlike `ak_call_try_recv`, this must not drain.
    let service = TestService::canned([Bytes::from_static(b"ok")]);
    let endpoint = serve(service);

    let client = Client::to(&endpoint, &[]);
    let call = client.start(METHOD_PATH, Kind::Unary, StartOptions::default());
    call.send_ok(b"ping");
    call.close_send_ok();
    let (_, first) = call.drain();

    let second = call.status().expect("the status should still be readable");
    assert_eq!(first.code, second.code);
    assert_eq!(first.message, second.message);
}

#[test]
fn a_path_the_server_does_not_serve_reports_the_server_status() {
    // The ABI routes an opaque path, so a path the server has no service for has to come back as the
    // server's own `Unimplemented` rather than as a local error. A different *service* name, not just
    // a different method: the test service answers every method under its own name, exactly as the
    // ABI answers every path.
    let endpoint = serve(TestService::canned([Bytes::from_static(b"ok")]));

    let client = Client::to(&endpoint, &[]);
    let call = client.start(
        "/armonik_transport_ffi.test.NoSuchService/Call",
        Kind::Unary,
        StartOptions::default(),
    );
    call.send_ok(b"ping");
    call.close_send_ok();

    let (messages, outcome) = call.drain();

    assert!(messages.is_empty());
    assert_eq!(outcome.code, GRPC_UNIMPLEMENTED);
}

#[test]
fn a_failed_unary_call_is_retried_until_it_succeeds() {
    let service =
        TestService::canned([Bytes::from_static(b"finally")]).failing_first(2, Code::Unavailable);
    let endpoint = serve(service.clone());

    let client = Client::to(&endpoint, &Client::quick_retry());
    let call = client.start(METHOD_PATH, Kind::Unary, StartOptions::default());
    call.send_ok(b"ping");
    call.close_send_ok();

    let (messages, outcome) = call.drain();

    assert_eq!(outcome.code, GRPC_OK, "the third attempt should succeed");
    assert_eq!(messages, vec![b"finally".to_vec()]);
    assert_eq!(
        service.attempts(),
        3,
        "the server should have seen three attempts, not one retried status"
    );
    assert_eq!(
        service.messages_received().len(),
        3,
        "each replay must resend the buffered request message"
    );
}

#[test]
fn a_retry_gives_up_at_the_configured_attempt_limit() {
    let service = TestService::canned([Bytes::from_static(b"unreachable")])
        .failing_first(10, Code::Unavailable);
    let endpoint = serve(service.clone());

    let client = Client::to(&endpoint, &Client::quick_retry());
    let call = client.start(METHOD_PATH, Kind::Unary, StartOptions::default());
    call.send_ok(b"ping");
    call.close_send_ok();

    let (_, outcome) = call.drain();

    assert_eq!(outcome.code, GRPC_UNAVAILABLE);
    assert_eq!(
        service.attempts(),
        3,
        "`MaxAttempts` is a total, not a number of retries on top of the first try"
    );
}

#[test]
fn a_status_outside_the_policy_is_not_retried() {
    let service = TestService::canned([Bytes::from_static(b"unreachable")])
        .failing_first(10, Code::InvalidArgument);
    let endpoint = serve(service.clone());

    let client = Client::to(&endpoint, &Client::quick_retry());
    let call = client.start(METHOD_PATH, Kind::Unary, StartOptions::default());
    call.send_ok(b"ping");
    call.close_send_ok();

    let (_, outcome) = call.drain();

    assert_eq!(outcome.code, GRPC_INVALID_ARGUMENT);
    assert_eq!(service.attempts(), 1);
}

#[test]
fn without_a_policy_the_first_failure_is_final() {
    let service = TestService::canned([Bytes::from_static(b"unreachable")])
        .failing_first(1, Code::Unavailable);
    let endpoint = serve(service.clone());

    // No `MaxAttempts`: retry is opt-in, and a client that did not ask for it must not get it.
    let client = Client::to(&endpoint, &[]);
    let call = client.start(METHOD_PATH, Kind::Unary, StartOptions::default());
    call.send_ok(b"ping");
    call.close_send_ok();

    let (_, outcome) = call.drain();

    assert_eq!(outcome.code, GRPC_UNAVAILABLE);
    assert_eq!(service.attempts(), 1);
}

#[test]
fn a_client_streaming_call_is_never_retried() {
    // The request stream is not buffered, so it cannot be reproduced. Retrying anyway would send a
    // truncated request and, worse, could duplicate a submission.
    let service = TestService::canned([Bytes::from_static(b"unreachable")])
        .failing_first(1, Code::Unavailable);
    let endpoint = serve(service.clone());

    let client = Client::to(&endpoint, &Client::quick_retry());
    let call = client.start(METHOD_PATH, Kind::ClientStreaming, StartOptions::default());
    call.send_ok(b"first");
    call.close_send_ok();

    let (_, outcome) = call.drain();

    assert_eq!(outcome.code, GRPC_UNAVAILABLE);
    assert_eq!(
        service.attempts(),
        1,
        "a client stream must not be replayed even though the status is retryable"
    );
}

#[test]
fn a_server_stream_that_already_yielded_a_message_is_not_retried() {
    // Replaying here would hand the caller the first message twice.
    let service = TestService::canned_then_error([Bytes::from_static(b"first")], Code::Unavailable);
    let endpoint = serve(service.clone());

    let client = Client::to(&endpoint, &Client::quick_retry());
    let call = client.start(METHOD_PATH, Kind::ServerStreaming, StartOptions::default());
    call.send_ok(b"start");
    call.close_send_ok();

    let (messages, outcome) = call.drain();

    assert_eq!(messages, vec![b"first".to_vec()]);
    assert_eq!(outcome.code, GRPC_UNAVAILABLE);
    assert_eq!(
        service.attempts(),
        1,
        "the stream had already produced a message, so it must not be replayed"
    );
}

#[test]
fn a_server_stream_that_failed_before_yielding_anything_is_retried() {
    // The mirror image of the test above, and the reason the message count is tracked at all: before
    // the first message, re-establishing the stream is invisible to the caller.
    let service =
        TestService::canned([Bytes::from_static(b"first")]).failing_first(1, Code::Unavailable);
    let endpoint = serve(service.clone());

    let client = Client::to(&endpoint, &Client::quick_retry());
    let call = client.start(METHOD_PATH, Kind::ServerStreaming, StartOptions::default());
    call.send_ok(b"start");
    call.close_send_ok();

    let (messages, outcome) = call.drain();

    assert_eq!(outcome.code, GRPC_OK);
    assert_eq!(messages, vec![b"first".to_vec()]);
    assert_eq!(service.attempts(), 2);
}

#[test]
fn one_client_serves_many_concurrent_calls() {
    // The whole point of a single multiplexed HTTP/2 connection. Also the shape of the .NET test that
    // already exists for the managed client, so a regression here would show up there too.
    const CALLS: usize = 32;

    let service = TestService::echo_each(Bytes::from_static(b"!"));
    let endpoint = serve(service.clone());
    let client = Client::to(&endpoint, &[]);

    let calls: Vec<_> = (0..CALLS)
        .map(|index| {
            let call = client.start(METHOD_PATH, Kind::Unary, StartOptions::default());
            call.send_ok(format!("call-{index:02}").as_bytes());
            call.close_send_ok();
            (index, call)
        })
        .collect();

    for (index, call) in calls {
        let (messages, outcome) = call.drain();
        assert_eq!(outcome.code, GRPC_OK, "call {index}");
        assert_eq!(
            messages,
            vec![format!("call-{index:02}!").into_bytes()],
            "each call must get its own reply, not another call's"
        );
    }
}

#[test]
fn a_call_outlives_the_client_it_was_started_from() {
    // Documented behaviour: `ak_call_start` takes what it needs out of the client, so freeing the
    // client only stops *new* calls. .NET disposes in whatever order the GC finalises, so this is not
    // a theoretical case.
    let service = TestService::canned([Bytes::from_static(b"pong")]);
    let endpoint = serve(service);

    let client = Client::to(&endpoint, &[]);
    let call = client.start(METHOD_PATH, Kind::Unary, StartOptions::default());
    call.send_ok(b"ping");
    call.close_send_ok();

    drop(client);

    let (messages, outcome) = call.drain();
    assert_eq!(messages, vec![b"pong".to_vec()]);
    assert_eq!(outcome.code, GRPC_OK);
}

#[test]
fn a_call_cannot_be_started_from_a_freed_client() {
    let endpoint = serve(TestService::canned([Bytes::from_static(b"ok")]));

    let client = Client::to(&endpoint, &[]);
    let dangling = client.free_early();

    // SAFETY: deliberately passing a freed handle, which the ABI documents as reported rather than
    // dereferenced — that is the whole assertion.
    let mut out: *mut armonik_transport_ffi::ak_call = std::ptr::null_mut();
    let started = unsafe {
        armonik_transport_ffi::ak_call_start(
            dangling,
            METHOD_PATH.as_ptr(),
            METHOD_PATH.len(),
            Kind::Unary as i32,
            std::ptr::null(),
            0,
            0,
            8,
            std::ptr::addr_of_mut!(out),
            std::ptr::null_mut(),
        )
    };

    assert_eq!(started, status::INVALID_HANDLE);
    assert!(out.is_null(), "no handle should have been produced");
}
