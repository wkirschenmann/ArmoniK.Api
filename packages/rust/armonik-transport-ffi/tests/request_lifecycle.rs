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

/// Open a request against `service` and close the request body straight away.
fn call(service: TestService) -> (Client, Request) {
    let endpoint = serve(service);
    let url = format!("{endpoint}{METHOD_PATH}");
    let client = Client::new(&endpoint);
    let request = Request::start(&client, &headers(&url)).expect("start the request");
    assert_eq!(request.close_send(), OK);
    (client, request)
}

#[test]
fn a_call_answers_with_headers_a_body_and_trailers() {
    let (_client, request) = call(TestService::canned([Bytes::from_static(b"pong")]));

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
    let (_client, request) = call(TestService::canned(replies.clone()));

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
    let (_client, request) = call(service);

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
    // the first read parks. `hang` would not do: it never returns a response at all.
    let (_client, request) = call(TestService::echo_each(""));
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
    let (_client, request) = call(TestService::echo_each(""));
    assert_eq!(
        request.close_send(),
        ak_status::AK_INVALID_STATE as i32,
        "the request body is already ended"
    );
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
fn releasing_a_request_before_it_completes_silences_it() {
    let endpoint = serve(TestService::hang());
    let url = format!("{endpoint}{METHOD_PATH}");
    let client = Client::new(&endpoint);
    let request = Request::start(&client, &headers(&url)).expect("start the request");
    assert_eq!(request.close_send(), OK);

    // Dropping the wrapper calls `ak_request_release`, which silences the callback. What must not
    // happen is a callback afterwards, into a context the caller has already given up - which in a
    // host application is the point at which a rooted object has gone.
    drop(request);
    std::thread::sleep(Duration::from_millis(300));
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
