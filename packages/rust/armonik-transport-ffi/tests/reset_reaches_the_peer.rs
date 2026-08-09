//! What aborting a request body actually puts on the wire.
//!
//! `request_lifecycle.rs` asks whether a cancelled request stops the handler waiting, and the answer
//! there can only be "the request stream ended": `tonic` turns a client RST_STREAM(CANCEL) on a
//! request stream into a clean end of stream on purpose, so a handler cannot tell that ending apart
//! from a client that half-closed. Which leaves one question open - is a reset being sent at all, or
//! does this side merely go quiet?
//!
//! These two answer it, without the ABI in between, by aborting the same kind of body with a
//! different reason. ENHANCE_YOUR_CALM has no special treatment anywhere, so it arrives at the
//! handler as a stream error; CANCEL, over the identical code path, arrives as a clean end. The
//! reset travels either way, and only the reading of it differs.

mod common;

use std::time::Duration;

use armonik_transport::reexports::h2;
use armonik_transport::reexports::http;
use armonik_transport::reexports::http_body_util::channel::Channel;
use armonik_transport::reexports::http_body_util::BodyExt;
use armonik_transport::reexports::hyper_util::client::legacy::Client;
use armonik_transport::reexports::hyper_util::rt::{TokioExecutor, TokioTimer};
use armonik_transport::HttpConfig;
use bytes::Bytes;
use common::server::{serve, TestService, METHOD_PATH};
use tonic::Code;

/// An error whose cause chain carries `reason`, which is where `hyper` looks for the RST_STREAM code
/// when a request body gives up.
#[derive(Debug)]
struct Abort(h2::Error);

impl std::fmt::Display for Abort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the request body was aborted")
    }
}

impl std::error::Error for Abort {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

/// One gRPC frame: a zero compression byte, a big-endian length, then the message.
fn frame(message: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8];
    out.extend_from_slice(&(message.len() as u32).to_be_bytes());
    out.extend_from_slice(message);
    out
}

/// Open a call on `service`, send one message, then abort the request body with `reason`, and answer
/// how the handler saw the request stream end.
fn abort_with(service: TestService, reason: h2::Reason) -> Option<Code> {
    let endpoint = serve(service.clone());
    let config: HttpConfig = serde_json::from_str(&format!(r#"{{"Endpoint": "{endpoint}"}}"#))
        .expect("the endpoint is an option of the vocabulary");
    let origin = http::Uri::try_from(&config).expect("the endpoint is a URI");
    let connector = armonik_transport::https_connector(config, origin).expect("a connector");

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("build a runtime");

    runtime.block_on(async move {
        let (mut sender, body) = Channel::<Bytes, Abort>::new(1);
        let request = http::Request::builder()
            .method(http::Method::POST)
            .uri(format!("{endpoint}{METHOD_PATH}"))
            .version(http::Version::HTTP_2)
            .header("content-type", "application/grpc")
            .header("te", "trailers")
            .body(
                body.map_err(|error| Box::new(error) as Box<dyn std::error::Error + Send + Sync>)
                    .boxed(),
            )
            .expect("a well-formed request");

        let client = Client::builder(TokioExecutor::new())
            .http2_only(true)
            .timer(TokioTimer::new())
            .pool_timer(TokioTimer::new())
            .build(connector);
        let response = client.request(request).await.expect("the call is answered");

        // One message through, so the handler is established and parked on the next one - which is
        // where the reset has to land.
        sender
            .send_data(Bytes::from(frame(b"live")))
            .await
            .expect("the first chunk is admitted");

        // Both halves: aborting the body is what puts the reset on the wire, and dropping the
        // response is what stops the connection waiting for the rest of it.
        sender.abort(Abort(h2::Error::from(reason)));
        drop(response);

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while service.stream_ends().is_empty() && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        *service
            .stream_ends()
            .first()
            .expect("the handler stopped reading the request stream")
    })
}

#[test]
fn a_body_aborted_with_enhance_your_calm_reaches_the_handler_as_that_stream_error() {
    // The witness. No layer treats ENHANCE_YOUR_CALM specially, so it comes back out as the code the
    // handler saw - which proves the abort really does send a RST_STREAM carrying the reason it was
    // given, rather than dropping the stream quietly.
    assert_eq!(
        abort_with(TestService::echo_each(""), h2::Reason::ENHANCE_YOUR_CALM),
        Some(Code::ResourceExhausted)
    );
}

#[test]
fn a_body_aborted_with_cancel_reaches_the_handler_as_a_clean_end_instead() {
    // The same code path, one constant different, and the handler sees no error at all. This is
    // `tonic` absorbing a client cancellation on a request stream by design, and it is why the
    // cancellation test in `request_lifecycle.rs` has to observe the end rather than the reason.
    assert_eq!(
        abort_with(TestService::echo_each(""), h2::Reason::CANCEL),
        None
    );
}
