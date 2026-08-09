//! The test service answers a gRPC call, driven by a plain HTTP/2 client.
//!
//! Nothing of this library is under test here, and that is the point: every other test in this
//! directory reads the fixture's answers as evidence about the ABI, so a fixture that had never been
//! shown to answer on its own would make a failure ambiguous. This one call goes out through the
//! same connector the ABI uses, without any of the ABI in between.

mod common;

use std::time::Duration;

use armonik_transport::reexports::http;
use armonik_transport::reexports::http_body_util::{BodyExt, Full};
use armonik_transport::reexports::hyper_util::client::legacy::Client;
use armonik_transport::reexports::hyper_util::rt::{TokioExecutor, TokioTimer};
use armonik_transport::HttpConfig;
use bytes::Bytes;
use common::server::{serve, TestService, METHOD_PATH};

/// One gRPC frame: a zero compression byte, a big-endian length, then the message.
fn frame(message: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8];
    out.extend_from_slice(&(message.len() as u32).to_be_bytes());
    out.extend_from_slice(message);
    out
}

#[test]
fn the_fixture_answers_a_grpc_call_over_the_connector_this_library_dials_with() {
    let endpoint = serve(TestService::canned([Bytes::from_static(b"pong")]));
    let config: HttpConfig = serde_json::from_str(&format!(r#"{{"Endpoint": "{endpoint}"}}"#))
        .expect("the endpoint is an option of the vocabulary");
    let origin = http::Uri::try_from(&config).expect("the endpoint is a URI");
    let connector = armonik_transport::https_connector(config, origin).expect("a connector");

    let request = http::Request::builder()
        .method(http::Method::POST)
        .uri(format!("{endpoint}{METHOD_PATH}"))
        .version(http::Version::HTTP_2)
        .header("content-type", "application/grpc")
        .header("te", "trailers")
        .body(Full::new(Bytes::from(frame(b"ping"))))
        .expect("a well-formed request");

    // A runtime of this test's own: the library's is not involved in what is being checked.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build a runtime");
    let (parts, body) = runtime
        .block_on(async {
            let client = Client::builder(TokioExecutor::new())
                .http2_only(true)
                .timer(TokioTimer::new())
                .pool_timer(TokioTimer::new())
                .build(connector);
            tokio::time::timeout(Duration::from_secs(10), client.request(request)).await
        })
        .expect("the fixture answers within ten seconds")
        .expect("the request reaches the fixture")
        .into_parts();

    assert_eq!(parts.status, http::StatusCode::OK);
    let collected = runtime
        .block_on(body.collect())
        .expect("the response body arrives whole");
    let trailers = collected.trailers().cloned();
    assert_eq!(collected.to_bytes(), Bytes::from(frame(b"pong")));

    // In the trailers rather than in the headers, because this handler answers before it fails or
    // succeeds: a trailers-only response is what a handler that refuses up front produces.
    let trailers = trailers.expect("a gRPC response carries trailers");
    assert_eq!(
        trailers.get("grpc-status").map(http::HeaderValue::as_bytes),
        Some(&b"0"[..])
    );
}
