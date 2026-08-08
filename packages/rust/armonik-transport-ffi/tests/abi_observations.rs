//! Measurements the spike owes the plan, rather than assertions about behaviour.
//!
//! Two of the ABI's open questions can only be answered with numbers and with the actual strings a
//! caller would see: how the response body is cut into `READ_DONE` events, and what a failed
//! `COMPLETED` really says. Run with `--nocapture` to read them.

mod common;

use armonik_transport_ffi::status;
use bytes::Bytes;
use common::abi::{Client, Event, Request};
use common::server::{serve, TestService, METHOD_PATH};

fn headers(url: &str) -> Vec<(&str, &str)> {
    vec![
        (":method", "POST"),
        (":url", url),
        ("content-type", "application/grpc"),
        ("te", "trailers"),
    ]
}

fn frame(message: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8];
    out.extend_from_slice(&(message.len() as u32).to_be_bytes());
    out.extend_from_slice(message);
    out
}

#[test]
fn how_a_large_response_is_cut_into_read_events() {
    // The open question is what a `READ_DONE` chunk is worth, which decides whether the C# side can
    // hand a chunk straight to its reader or has to keep the remainder. It has to keep it: a chunk
    // is a DATA frame's worth of body, and a gRPC message spans many.
    let payload = vec![b'x'; 16 * 1024 * 1024];
    let endpoint = serve(TestService::canned([Bytes::from(payload.clone())]));
    let url = format!("{endpoint}{METHOD_PATH}");
    let client = Client::new(&endpoint);
    let request = Request::start(&client, &headers(&url)).expect("start the request");

    assert_eq!(request.write(&frame(b"go")), status::OK);
    assert_eq!(request.next_event(), Event::WriteDone);
    assert_eq!(request.close_send(), status::OK);
    let Event::Headers(_) = request.next_event() else {
        panic!("expected the response headers");
    };

    let mut sizes = Vec::new();
    loop {
        assert_eq!(request.read(), status::OK);
        match request.next_event() {
            Event::Read(chunk) => sizes.push(chunk.len()),
            Event::Completed { code, .. } => {
                assert_eq!(code, status::OK);
                break;
            }
            other => panic!("expected a read or a completion, got {other:?}"),
        }
    }

    let total: usize = sizes.iter().sum();
    let smallest = sizes.iter().min().copied().unwrap_or(0);
    let largest = sizes.iter().max().copied().unwrap_or(0);
    println!(
        "READ_DONE for a {} MiB message: {} chunks, {} to {} bytes, {} bytes in all, first five {:?}",
        payload.len() / 1024 / 1024,
        sizes.len(),
        smallest,
        largest,
        total,
        &sizes[..sizes.len().min(5)]
    );
    assert_eq!(total, frame(&payload).len());
}

#[test]
fn what_a_failed_completion_actually_says() {
    // The plan asks for the real strings rather than a taxonomy invented in advance, because they
    // are what a host application ends up putting in a log.
    let refused = {
        let client = Client::new("http://127.0.0.1:1");
        let request = Request::start(
            &client,
            &headers("http://127.0.0.1:1/armonik_transport_ffi.test.Raw/Call"),
        )
        .expect("start the request");
        assert_eq!(request.close_send(), status::OK);
        let terminal = request.next_event();
        let (code, message) = terminal.expect_completed();
        format!("{code} {message}")
    };
    println!("connection refused -> {refused}");

    let cancelled = {
        let endpoint = serve(TestService::echo_each(""));
        let url = format!("{endpoint}{METHOD_PATH}");
        let client = Client::new(&endpoint);
        let request = Request::start(&client, &headers(&url)).expect("start the request");
        assert_eq!(request.cancel(), status::OK);
        let terminal = request.next_event();
        let (code, message) = terminal.expect_completed();
        format!("{code} {message}")
    };
    println!("cancelled          -> {cancelled}");

    let timed_out = {
        let endpoint = serve(TestService::hang());
        let url = format!("{endpoint}{METHOD_PATH}");
        let client = Client::try_new(&format!(
            r#"{{"Endpoint": "{endpoint}", "Timeout": "300ms"}}"#
        ))
        .expect("create the client");
        let request = Request::start(&client, &headers(&url)).expect("start the request");
        assert_eq!(request.close_send(), status::OK);
        let terminal = request.next_event();
        let (code, message) = terminal.expect_completed();
        format!("{code} {message}")
    };
    println!("timed out          -> {timed_out}");

    let bad_endpoint = Client::try_new(r#"{"Endpoint": "https://localhost/", "CaCert": "nope.pem"}"#)
        .err()
        .expect("an unreadable CA file is refused");
    println!(
        "bad configuration  -> {} {}",
        bad_endpoint.0, bad_endpoint.1
    );
}
