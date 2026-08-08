//! Does a cancelled request actually reach the server, or does it just go quiet on this side?
//!
//! Separate from `request_lifecycle.rs` because it asks a different question. There, cancelling is
//! checked from the client's side: one `COMPLETED`, nothing after it. Here the only thing that
//! counts is what the peer saw, which is what decides whether a server stops working on an
//! abandoned call or keeps going.
//!
//! The observation is indirect, and has to be. `tonic` turns a client RST_STREAM(CANCEL) on a
//! request stream into a clean end of stream on purpose - see the
//! `direction == Request && code == Cancelled` arm of `tonic::codec::decode` - so no handler can
//! see a cancellation as such. The service therefore counts request streams that end on a method
//! whose client never half-closes, where an end can only mean the peer went away.

mod common;

use armonik_transport_ffi::status;
use common::abi::{Client, Event, Request};
use common::spike::serve_spike;

/// The headers of a gRPC-shaped request to `url`.
fn headers(url: &str) -> Vec<(&str, &str)> {
    vec![
        (":method", "POST"),
        (":url", url),
        ("content-type", "application/grpc"),
        ("te", "trailers"),
    ]
}

/// One gRPC frame around a message.
fn frame(message: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8];
    out.extend_from_slice(&(message.len() as u32).to_be_bytes());
    out.extend_from_slice(message);
    out
}

/// Ask the service how many cancellations it has seen, and the last stream error code.
fn observed(client: &Client, endpoint: &str) -> (u32, u32) {
    let url = format!("{endpoint}/armonik_transport_ffi.test.Raw/CancelsObserved");
    let request = Request::start(client, &headers(&url)).expect("start the request");
    assert_eq!(
        request.write(&frame(&common::spike::encode_echo(b""))),
        status::OK
    );
    assert_eq!(request.next_event(), Event::WriteDone);
    assert_eq!(request.close_send(), status::OK);
    let Event::Headers(_) = request.next_event() else {
        panic!("expected the response headers");
    };

    let (body, terminal) = request.drain();
    assert_eq!(terminal.expect_completed().0, status::OK);

    // Strip the gRPC frame, then the EchoMsg, to get "<count>:<last code>".
    let message = &body[5..];
    let payload = common::spike::decode_echo(message).expect("an EchoMsg");
    let text = String::from_utf8_lossy(&payload).into_owned();
    let (count, last) = text.split_once(':').expect("count:last");
    (
        count.parse().expect("a count"),
        last.parse().expect("a code"),
    )
}

#[test]
fn cancelling_a_request_stops_the_server_waiting_on_it() {
    let address = serve_spike(0);
    let endpoint = format!("http://{address}");
    let client = Client::new(&endpoint);

    let (before, _) = observed(&client, &endpoint);

    {
        let url = format!("{endpoint}/armonik_transport_ffi.test.Raw/CancelWatch");
        let request = Request::start(&client, &headers(&url)).expect("start the request");

        // Send one message and read its reply, so the call is established and the handler is
        // parked on the next `stream.message()` - which is where a reset has to land.
        assert_eq!(
            request.write(&frame(&common::spike::encode_echo(b"live"))),
            status::OK
        );
        assert_eq!(request.next_event(), Event::WriteDone);
        let Event::Headers(_) = request.next_event() else {
            panic!("expected the response headers");
        };
        assert_eq!(request.read(), status::OK);
        let Event::Read(_) = request.next_event() else {
            panic!("expected the echoed reply");
        };

        assert_eq!(request.cancel(), status::OK);
        let terminal = request.next_event();
        assert_eq!(terminal.expect_completed().0, status::CANCELLED);
    }

    // The reset travels on its own; the count is not expected to have moved by the time the
    // cancelled request has completed on this side.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut after = before;
    let mut last = u32::MAX;
    while after == before && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let seen = observed(&client, &endpoint);
        after = seen.0;
        last = seen.1;
    }

    assert!(
        after > before,
        "the server was left waiting on the cancelled request: {before} then {after},          last stream error {last}"
    );
}
