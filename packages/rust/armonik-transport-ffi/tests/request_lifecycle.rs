//! The reactor, end to end against a real gRPC server.
//!
//! Every test body is synchronous: the ABI is driven from the test's own thread while the server
//! runs on a runtime of its own. Driving it from inside an `async` block would mean occupying a
//! thread the runtime needs in order to answer.

mod common;

use std::time::Duration;

use armonik_transport_ffi::status::ak_status;
use bytes::Bytes;
use common::abi::{Client, Event, Request, OK};
use common::server::{serve, TestService, METHOD_PATH};

/// The headers of a gRPC-shaped request to `url`.
fn headers(url: &str) -> Vec<(&str, &str)> {
    vec![
        (":method", "POST"),
        (":url", url),
        ("content-type", "application/grpc"),
        ("te", "trailers"),
    ]
}

/// One gRPC frame: a zero compression byte, a big-endian length, then the message.
fn frame(message: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8];
    out.extend_from_slice(&(message.len() as u32).to_be_bytes());
    out.extend_from_slice(message);
    out
}

/// Open a request against `service`, send `messages` and end the request body.
fn call(service: TestService, messages: &[&[u8]]) -> (Client, Request) {
    let endpoint = serve(service);
    let url = format!("{endpoint}{METHOD_PATH}");
    let client = Client::new(&endpoint);
    let request = Request::start(&client, &headers(&url)).expect("start the request");

    for message in messages {
        assert_eq!(request.write(&frame(message)), OK);
        assert_eq!(request.next_event(), Event::WriteDone);
    }
    assert_eq!(request.close_send(), OK);
    (client, request)
}

/// Open a request against `service` and leave the request body open.
///
/// What a test needs when the response must not be allowed to end on its own: an `echo_each` handler
/// answers with headers straight away and then reads, so as long as this side never ends its request
/// body, the handler stays parked and nothing arrives unasked.
fn open(service: TestService) -> (Client, Request) {
    let endpoint = serve(service);
    let url = format!("{endpoint}{METHOD_PATH}");
    let client = Client::new(&endpoint);
    let request = Request::start(&client, &headers(&url)).expect("start the request");
    (client, request)
}

#[test]
fn a_call_answers_with_headers_a_body_and_trailers() {
    let (_client, request) = call(
        TestService::canned([Bytes::from_static(b"pong")]),
        &[b"ping"],
    );

    assert_eq!(request.next_event().expect_http_status(), "200");

    let (body, terminal) = request.drain();
    assert_eq!(body, frame(b"pong"));

    let Event::Completed { code, trailers, .. } = &terminal else {
        panic!("expected the completion, got {terminal:?}");
    };
    assert_eq!(*code, OK);
    let status_trailer = trailers
        .iter()
        .find(|(key, _)| key == b"grpc-status")
        .expect("a gRPC response carries a grpc-status trailer");
    assert_eq!(status_trailer.1, b"0");
}

#[test]
fn a_server_streaming_call_delivers_every_message_in_order() {
    let replies: Vec<Bytes> = (0..5)
        .map(|index| Bytes::from(format!("reply-{index}")))
        .collect();
    let (_client, request) = call(TestService::canned(replies.clone()), &[b"go"]);

    assert_eq!(request.next_event().expect_http_status(), "200");
    let (body, terminal) = request.drain();

    let expected: Vec<u8> = replies.iter().flat_map(|reply| frame(reply)).collect();
    assert_eq!(body, expected, "the messages arrive whole and in order");
    assert_eq!(terminal.expect_completed().0, OK);
}

#[test]
fn a_failing_call_reports_its_status_where_the_server_put_it() {
    let service = TestService::canned([Bytes::from_static(b"unused")])
        .failing_first(1, tonic::Code::PermissionDenied)
        .with_failure_trailer("x-why", "because");
    let (_client, request) = call(service, &[b"ping"]);

    // A handler that fails before writing anything produces a trailers-only response: the status is
    // in the initial headers, and the body ends immediately.
    let headers = request.next_event();
    assert_eq!(headers.expect_http_status(), "200");
    let Event::Headers(pairs) = &headers else {
        unreachable!("checked just above")
    };
    let grpc_status = pairs.iter().find(|(key, _)| key == b"grpc-status");

    let (body, terminal) = request.drain();
    assert!(body.is_empty(), "a trailers-only response has no body");
    let Event::Completed { code, trailers, .. } = &terminal else {
        panic!("expected the completion, got {terminal:?}");
    };
    assert_eq!(*code, OK, "the transport succeeded; gRPC refused");

    // Wherever it ended up, the status has to be readable: in the headers for a trailers-only
    // response, in the trailers otherwise. Which one is what a caller has to cope with.
    let found = grpc_status
        .or_else(|| trailers.iter().find(|(key, _)| key == b"grpc-status"))
        .expect("the gRPC status is somewhere");
    assert_eq!(String::from_utf8_lossy(&found.1), "7");
}

#[test]
fn a_second_read_armed_before_the_first_answers_is_refused() {
    // `echo_each` answers with headers before it has anything to say, so the response is open and
    // the first read parks. `hang` would not do: it never returns a response at all. The request
    // body stays open, or the handler would end its reply and the first read would answer.
    let (_client, request) = open(TestService::echo_each(""));
    assert_eq!(request.next_event().expect_http_status(), "200");

    assert_eq!(request.read(), OK);
    assert_eq!(
        request.read(),
        ak_status::AK_INVALID_STATE as i32,
        "one armed read at a time is the rule the whole reactor rests on"
    );
}

#[test]
fn ending_the_request_body_twice_is_refused() {
    let (_client, request) = call(TestService::echo_each(""), &[]);
    assert_eq!(
        request.close_send(),
        ak_status::AK_INVALID_STATE as i32,
        "the request body is already ended"
    );
}

#[test]
fn a_write_after_the_request_body_is_ended_is_refused() {
    let (_client, request) = call(TestService::echo_each(""), &[b"ping"]);
    assert_eq!(
        request.write(b"late"),
        ak_status::AK_INVALID_STATE as i32,
        "the request body is already ended"
    );
}

#[test]
fn a_second_write_armed_before_the_first_is_admitted_is_refused() {
    // `hang_without_reading` never opens its flow-control window, so the first chunk is never
    // admitted and its event never comes: the write stays armed for as long as the test needs.
    let endpoint = serve(TestService::hang_without_reading());
    let url = format!("{endpoint}{METHOD_PATH}");
    let client = Client::new(&endpoint);
    let request = Request::start(&client, &headers(&url)).expect("start the request");

    // Enough to fill the initial window and then some, so the second chunk cannot slip through.
    let payload = vec![b'x'; 1024 * 1024];
    assert_eq!(request.write(&payload), OK);
    assert_eq!(
        request.write(b"second"),
        ak_status::AK_INVALID_STATE as i32,
        "one armed write at a time"
    );
    assert_eq!(
        request.close_send(),
        ak_status::AK_INVALID_STATE as i32,
        "the body cannot be ended under an armed write either"
    );
}

#[test]
fn a_client_streaming_call_sends_every_message_before_the_response() {
    let service = TestService::canned([Bytes::from_static(b"counted")]);
    let messages: Vec<Vec<u8>> = (0..4)
        .map(|index| format!("m{index}").into_bytes())
        .collect();
    let borrowed: Vec<&[u8]> = messages.iter().map(Vec::as_slice).collect();
    let (_client, request) = call(service.clone(), &borrowed);

    assert_eq!(request.next_event().expect_http_status(), "200");
    let (body, terminal) = request.drain();
    assert_eq!(body, frame(b"counted"));
    assert_eq!(terminal.expect_completed().0, OK);

    let received = service.messages_received();
    assert_eq!(received.len(), 1);
    assert_eq!(
        received[0].len(),
        messages.len(),
        "every message the caller wrote reaches the handler"
    );
}

#[test]
fn a_bidirectional_call_interleaves_strictly() {
    // The proof that neither side buffers: the server answers each message as it arrives, and this
    // test refuses to send the next one until it has read the previous reply. If anything held the
    // request body back until it closed, this would sit here until the patience runs out.
    let endpoint = serve(TestService::echo_each("!"));
    let url = format!("{endpoint}{METHOD_PATH}");
    let client = Client::new(&endpoint);
    let request = Request::start(&client, &headers(&url)).expect("start the request");

    assert_eq!(request.write(&frame(b"one")), OK);
    assert_eq!(request.next_event(), Event::WriteDone);
    assert_eq!(request.next_event().expect_http_status(), "200");

    for message in [&b"one"[..], b"two", b"three"] {
        let mut seen = Vec::new();
        while seen.len() < frame(message).len() + 1 {
            assert_eq!(request.read(), OK);
            match request.next_event() {
                Event::Read(chunk) => seen.extend_from_slice(&chunk),
                other => panic!("expected a reply to {message:?}, got {other:?}"),
            }
        }
        let mut expected = message.to_vec();
        expected.push(b'!');
        assert_eq!(seen, frame(&expected));

        if message != b"three" {
            let next = if message == b"one" {
                &b"two"[..]
            } else {
                b"three"
            };
            assert_eq!(request.write(&frame(next)), OK);
            assert_eq!(request.next_event(), Event::WriteDone);
        }
    }

    assert_eq!(request.close_send(), OK);
    let (_, terminal) = request.drain();
    assert_eq!(terminal.expect_completed().0, OK);
}

#[test]
fn a_large_message_survives_the_flow_control_window_in_both_directions() {
    // 16 MiB, well past the 64 KiB initial HTTP/2 window: the request body has to be admitted a
    // window at a time through a queue that holds one chunk, and the response has to come back as
    // however many chunks the connection chose to split it into.
    let payload = vec![b'x'; 16 * 1024 * 1024];
    let (_client, request) = call(
        TestService::canned([Bytes::from(payload.clone())]),
        &[&payload],
    );

    assert_eq!(request.next_event().expect_http_status(), "200");
    let (body, terminal) = request.drain();
    assert_eq!(body.len(), frame(&payload).len());
    assert_eq!(body, frame(&payload));
    assert_eq!(terminal.expect_completed().0, OK);
}

#[test]
fn the_configured_user_agent_reaches_the_server() {
    let service = TestService::canned([Bytes::from_static(b"pong")]);
    let endpoint = serve(service.clone());
    let url = format!("{endpoint}{METHOD_PATH}");
    let client = Client::try_new(&format!(
        r#"{{"Endpoint": "{endpoint}", "UserAgent": "armonik-test/9.9"}}"#
    ))
    .expect("create the client");

    let request = Request::start(&client, &headers(&url)).expect("start the request");
    assert_eq!(request.close_send(), OK);
    assert_eq!(request.next_event().expect_http_status(), "200");
    assert_eq!(request.drain().1.expect_completed().0, OK);

    let seen = service.headers_received();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].get("user-agent").map(|value| value.as_bytes()),
        Some(&b"armonik-test/9.9"[..]),
        "an option that is parsed and then dropped on the floor is the failure the ledger exists \
         to catch"
    );
}

#[test]
fn a_user_agent_of_the_caller_s_own_wins_over_the_configured_one() {
    let service = TestService::canned([Bytes::from_static(b"pong")]);
    let endpoint = serve(service.clone());
    let url = format!("{endpoint}{METHOD_PATH}");
    let client = Client::try_new(&format!(
        r#"{{"Endpoint": "{endpoint}", "UserAgent": "from-the-configuration"}}"#
    ))
    .expect("create the client");

    let mut with_agent = headers(&url);
    with_agent.push(("user-agent", "from-the-request"));
    let request = Request::start(&client, &with_agent).expect("start the request");
    assert_eq!(request.close_send(), OK);
    assert_eq!(request.next_event().expect_http_status(), "200");
    assert_eq!(request.drain().1.expect_completed().0, OK);

    let seen = service.headers_received();
    assert_eq!(
        seen[0].get("user-agent").map(|value| value.as_bytes()),
        Some(&b"from-the-request"[..]),
        "the configured agent is a default, not an override"
    );
}

#[test]
fn a_request_outlives_the_client_it_was_started_on() {
    let endpoint = serve(TestService::canned([Bytes::from_static(b"pong")]));
    let url = format!("{endpoint}{METHOD_PATH}");
    let client = Client::new(&endpoint);
    let request = Request::start(&client, &headers(&url)).expect("start the request");
    assert_eq!(request.close_send(), OK);

    // The pool is reference-counted into the request, so this is not the end of it.
    drop(client);

    assert_eq!(request.next_event().expect_http_status(), "200");
    let (body, terminal) = request.drain();
    assert_eq!(body, frame(b"pong"));
    assert_eq!(terminal.expect_completed().0, OK);
}

#[test]
fn a_request_that_cannot_connect_completes_with_the_reason() {
    let client = Client::new("http://127.0.0.1:1");
    let request = Request::start(
        &client,
        &headers("http://127.0.0.1:1/armonik_transport_ffi.test.Raw/Call"),
    )
    .expect("start the request");
    assert_eq!(request.close_send(), OK);

    let terminal = request.next_event();
    let (code, message) = terminal.expect_completed();
    assert_eq!(code, ak_status::AK_CONNECTION_FAILED as i32);
    assert!(
        !message.is_empty(),
        "the caller has nothing but this string to diagnose with"
    );
}

#[test]
fn cancelling_after_the_headers_completes_the_request_once() {
    // The request body is left open, so the handler is parked on the next message and the response
    // cannot end on its own: what the completion reports can only be the cancellation.
    let (_client, request) = open(TestService::echo_each(""));
    assert_eq!(request.next_event().expect_http_status(), "200");

    assert_eq!(request.read(), OK);
    assert_eq!(request.cancel(), OK);

    let terminal = request.next_event();
    let (code, message) = terminal.expect_completed();
    assert_eq!(code, ak_status::AK_CANCELLED as i32, "{message}");

    assert_eq!(
        request.try_next_event(Duration::from_millis(200)),
        None,
        "the completion is terminal: nothing may follow it"
    );
}

#[test]
fn cancelling_before_the_headers_completes_the_request_once() {
    // Nothing has been answered, so there is no response body to drop: what stops the attempt is
    // dropping the request future itself.
    let endpoint = serve(TestService::hang_without_reading());
    let url = format!("{endpoint}{METHOD_PATH}");
    let client = Client::new(&endpoint);
    let request = Request::start(&client, &headers(&url)).expect("start the request");

    assert_eq!(request.cancel(), OK);
    let terminal = request.next_event();
    assert_eq!(
        terminal.expect_completed().0,
        ak_status::AK_CANCELLED as i32
    );
    assert_eq!(request.try_next_event(Duration::from_millis(200)), None);
}

#[test]
fn cancelling_a_request_that_has_already_completed_changes_nothing() {
    let (_client, request) = call(
        TestService::canned([Bytes::from_static(b"pong")]),
        &[b"ping"],
    );
    assert_eq!(request.next_event().expect_http_status(), "200");
    assert_eq!(request.drain().1.expect_completed().0, OK);

    // The task is gone and its command channel with it, which is not an error: the request is over,
    // which is what the caller was asking for.
    assert_eq!(request.cancel(), OK);
    assert_eq!(request.try_next_event(Duration::from_millis(200)), None);
}

#[test]
fn cancelling_stops_the_server_waiting_on_the_request() {
    // The peer's side of a cancellation, and the only form in which a handler can see it. `tonic`
    // turns a client RST_STREAM(CANCEL) on a request stream into a clean end of stream on purpose,
    // so what is observable is not the reason but the fact: the handler stops waiting. This client
    // never ends its request body, so an end can only mean the peer went away.
    let service = TestService::echo_each("");
    let endpoint = serve(service.clone());
    let url = format!("{endpoint}{METHOD_PATH}");
    let client = Client::new(&endpoint);
    let request = Request::start(&client, &headers(&url)).expect("start the request");

    // One message through and its reply read, so the call is established and the handler is parked
    // on the next message - which is where a reset has to land.
    assert_eq!(request.write(&frame(b"live")), OK);
    assert_eq!(request.next_event(), Event::WriteDone);
    assert_eq!(request.next_event().expect_http_status(), "200");
    assert_eq!(request.read(), OK);
    let Event::Read(_) = request.next_event() else {
        panic!("expected the echoed reply");
    };
    assert!(
        service.stream_ends().is_empty(),
        "the handler is still reading"
    );

    assert_eq!(request.cancel(), OK);
    assert_eq!(
        request.next_event().expect_completed().0,
        ak_status::AK_CANCELLED as i32
    );

    // The reset travels on its own, so the handler is not expected to have noticed by the time this
    // side has completed.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while service.stream_ends().is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        service.stream_ends(),
        vec![None],
        "the server was left waiting on a request nobody is on the other end of"
    );
}

#[test]
fn releasing_a_request_before_it_completes_silences_it() {
    let endpoint = serve(TestService::hang());
    let url = format!("{endpoint}{METHOD_PATH}");
    let client = Client::new(&endpoint);
    let request = Request::start(&client, &headers(&url)).expect("start the request");
    assert_eq!(request.close_send(), OK);

    // Dropping the wrapper calls `ak_request_release`, which silences the callback and cancels. What
    // must not happen is a callback afterwards, into a context the caller has already given up -
    // which in a host application is the point at which a rooted object has gone.
    drop(request);
    std::thread::sleep(Duration::from_millis(300));
}

#[test]
#[serial_test::serial]
fn repeated_requests_do_not_grow_the_runtime() {
    // The leak check the ABI has no other way to make. A request abandoned before it completed
    // leaves nothing visible from the outside: its driving task simply never ends. Equal batches,
    // counted after each, is what catches that.
    //
    // The comparison is one-sided, and that is not laziness. The count includes the connection tasks
    // of the pool, which come and go as connections are opened and retired, so it may well fall
    // between two batches. Only growth proportional to the batch is a leak.
    let endpoint = serve(TestService::canned([Bytes::from_static(b"pong")]));
    let url = format!("{endpoint}{METHOD_PATH}");
    let client = Client::new(&endpoint);

    let batch = || {
        for _ in 0..BATCH {
            let request = Request::start(&client, &headers(&url)).expect("start the request");
            assert_eq!(request.write(&frame(b"ping")), OK);
            assert_eq!(request.next_event(), Event::WriteDone);
            assert_eq!(request.close_send(), OK);
            assert_eq!(request.next_event().expect_http_status(), "200");
            assert_eq!(request.drain().1.expect_completed().0, OK);
        }
        settled_tasks()
    };

    batch(); // Warm-up: the first batch is also what opens the connection.
    let after_first = batch();
    let after_second = batch();
    assert!(
        after_second <= after_first,
        "the second batch left tasks behind: {after_first} then {after_second}"
    );
}

#[test]
#[serial_test::serial]
fn cancelled_requests_do_not_grow_the_runtime() {
    // The case that matters most: a cancelled request has to run its task to an end rather than park
    // forever on a stream nobody will ever answer. A release before the completion rests on the same
    // thing, being a cancel with the callback silenced.
    let endpoint = serve(TestService::echo_each(""));
    let url = format!("{endpoint}{METHOD_PATH}");
    let client = Client::new(&endpoint);

    let batch = || {
        for _ in 0..BATCH {
            let request = Request::start(&client, &headers(&url)).expect("start the request");
            assert_eq!(request.write(&frame(b"ping")), OK);
            assert_eq!(request.next_event(), Event::WriteDone);
            assert_eq!(request.next_event().expect_http_status(), "200");
            assert_eq!(request.read(), OK);
            assert_eq!(request.cancel(), OK);
            // The armed read may answer before the cancel lands; either way the last event is the
            // completion.
            loop {
                if let Event::Completed { .. } = request.next_event() {
                    break;
                }
            }
        }
        settled_tasks()
    };

    batch();
    let after_first = batch();
    let after_second = batch();
    assert!(
        after_second <= after_first,
        "cancelled requests parked forever: {after_first} then {after_second}"
    );
}

/// How many requests a leak batch makes. Large enough that one leaked task per request would show
/// well above the pool coming and going on its own.
const BATCH: usize = 8;

/// The live task count, once the tasks that have just finished have actually been dropped.
fn settled_tasks() -> usize {
    // A task is released a moment after its future returns, not during.
    std::thread::sleep(Duration::from_millis(300));
    armonik_transport_ffi::runtime::alive_tasks()
}

#[test]
fn a_request_released_from_another_thread_while_calls_are_in_flight_is_not_a_use_after_free() {
    // The race a set of live addresses cannot close: the check that a handle is live and the use of
    // what it points at are two moments, so a release landing between them would deallocate an
    // object a call is halfway through. A host application whose UI thread abandons a request while
    // a pool thread is still writing to it does exactly this. Run under a sanitiser, a regression
    // here is a use-after-free; run plainly, it is at worst a crash.
    let endpoint = serve(TestService::echo_each(""));
    let url = format!("{endpoint}{METHOD_PATH}");

    for _ in 0..64 {
        let client = Client::new(&endpoint);
        let request = Request::start(&client, &headers(&url)).expect("start the request");
        let handle = request.raw();

        let caller = std::thread::spawn(move || {
            for _ in 0..200 {
                // Whatever these return - `AK_OK`, `AK_INVALID_STATE`, `AK_INVALID_HANDLE` - is
                // fine. What is under test is that they return at all, rather than touching freed
                // memory.
                let _ = handle.read();
                let _ = handle.write(b"x");
                let _ = handle.cancel();
            }
        });

        // Racing the loop above on purpose: this is the `ak_request_release`.
        drop(request);
        caller.join().expect("the caller thread should not crash");
    }
}

#[test]
fn a_request_without_a_url_is_refused_before_anything_is_started() {
    let client = Client::new("http://127.0.0.1:1");
    let error = Request::start(&client, &[(":method", "POST")]).expect_err("no `:url`");
    assert_eq!(error.0, ak_status::AK_INVALID_STATE as i32);
    assert!(error.1.contains(":url"), "{}", error.1);
}

#[test]
fn a_relative_url_is_refused_because_the_pool_keys_on_the_authority() {
    let client = Client::new("http://127.0.0.1:1");
    let error = Request::start(&client, &[(":method", "POST"), (":url", "/only/a/path")])
        .expect_err("a relative URL");
    assert_eq!(error.0, ak_status::AK_INVALID_STATE as i32);
    assert!(error.1.contains("absolute"), "{}", error.1);
}

#[test]
fn a_request_refused_at_the_start_never_delivers_an_event() {
    // The other half of the rule the completion event rests on: no status but `AK_OK` from
    // `ak_request_start` may be followed by anything at all, or a caller that released its context
    // on the failure is called back into freed memory.
    let client = Client::new("http://127.0.0.1:1");
    let error = Request::start(&client, &[(":url", "http://127.0.0.1:1/")]).expect_err("no method");
    assert_eq!(error.0, ak_status::AK_INVALID_STATE as i32);
    assert!(error.1.contains(":method"), "{}", error.1);
}
