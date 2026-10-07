//! `Http2.Send.MaxHeaderListSize`: a call whose request headers weigh more than the limit ends
//! RESOURCE_EXHAUSTED on the channel, before a connection is taken and with nothing sent.

mod common;

use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use armonik_transport::grpc::{CallStartOptions, GrpcChannelConfig, GrpcStatusCode};
use armonik_transport::http2::TransportConfig;
use armonik_transport::reexports::hyper;
use armonik_transport::reexports::hyper_util::rt::{TokioExecutor as HyperTokio, TokioIo};
use bytes::Bytes;
use common::echo::{answer, channel_with, loopback, unary, ECHO};
use http::Uri;

/// An HTTP/2 server that counts what reaches it, and announces `max_header_list_size` when given
/// one.
struct Counting {
    endpoint: String,
    connections: Arc<AtomicUsize>,
    requests: Arc<AtomicUsize>,
}

async fn counting(max_header_list_size: Option<u32>) -> Counting {
    let (listener, endpoint) = loopback().await;
    let connections = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(AtomicUsize::new(0));
    let (accepted, served) = (connections.clone(), requests.clone());
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            accepted.fetch_add(1, Ordering::SeqCst);
            let served = served.clone();
            tokio::spawn(async move {
                let service = hyper::service::service_fn(move |request| {
                    served.fetch_add(1, Ordering::SeqCst);
                    async { Ok::<_, Infallible>(answer(request).await) }
                });
                let mut builder = hyper::server::conn::http2::Builder::new(HyperTokio::new());
                if let Some(max) = max_header_list_size {
                    builder.max_header_list_size(max);
                }
                let _ = builder
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    Counting {
        endpoint,
        connections,
        requests,
    }
}

fn limited(endpoint: &str, limit: Option<usize>) -> armonik_transport::grpc::GrpcChannel {
    let mut transport = TransportConfig::new(Uri::try_from(endpoint).expect("an endpoint"));
    transport.connect_timeout = Duration::from_secs(5);
    transport.http2.max_header_list_size = limit;
    channel_with(GrpcChannelConfig::new(transport)).expect("a channel")
}

/// A call with `padding` bytes of metadata, and the code it ends with, and what it said.
async fn padded(
    channel: &armonik_transport::grpc::GrpcChannel,
    padding: usize,
) -> (GrpcStatusCode, String) {
    let mut options = CallStartOptions::new(ECHO);
    options
        .metadata
        .append_ascii("x-pad", &"a".repeat(padding))
        .expect("a plain metadata entry");
    let (_, _, status) = unary(channel, options, Bytes::from_static(b"hello")).await;
    (status.code, status.message)
}

#[tokio::test]
async fn a_request_past_the_limit_ends_resource_exhausted_with_nothing_sent() {
    let server = counting(None).await;
    let channel = limited(&server.endpoint, Some(1000));

    let (code, message) = padded(&channel, 2000).await;
    assert_eq!(code, GrpcStatusCode::ResourceExhausted, "{message}");
    assert!(message.contains("header list"), "{message}");
    assert_eq!(
        server.connections.load(Ordering::SeqCst),
        0,
        "a refused call takes no connection and dials none"
    );

    let (code, message) = padded(&channel, 10).await;
    assert_eq!(code, GrpcStatusCode::Ok, "{message}");
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
}

/// With no limit nothing is refused for its headers, which a server then judges alone.
#[tokio::test]
async fn no_limit_refuses_nothing() {
    let server = counting(None).await;
    let channel = limited(&server.endpoint, None);

    let (code, message) = padded(&channel, 15_000).await;
    assert_eq!(code, GrpcStatusCode::Ok, "{message}");
}

/// Whether a call with `padding` bytes of metadata is let through under `limit`. A refused call
/// never reaches the server, so one server serves every probe.
async fn admitted(endpoint: &str, limit: usize, padding: usize) -> bool {
    let channel = limited(endpoint, Some(limit));
    padded(&channel, padding).await.0 != GrpcStatusCode::ResourceExhausted
}

/// The limit weighs a request as h2 does. The smallest limit that lets a request through is its
/// weight W, since the channel admits a list of at most W. An h2 server announcing W refuses a
/// list of W, so the test announces W + 1, which serves it, and W - 1, which refuses it. The unit
/// test of `header_list_size` pins the sum itself.
#[tokio::test]
async fn the_limit_weighs_a_request_as_h2_does() {
    let padding = 300;
    let probe = counting(None).await;
    let (mut refused, mut admits) = (0, 16_384);
    assert!(!admitted(&probe.endpoint, refused + 1, padding).await);
    assert!(admitted(&probe.endpoint, admits, padding).await);
    while admits - refused > 1 {
        let middle = (refused + admits) / 2;
        if admitted(&probe.endpoint, middle, padding).await {
            admits = middle;
        } else {
            refused = middle;
        }
    }
    let weight = admits;

    for (announced, served) in [(weight as u32 + 1, true), (weight as u32 - 1, false)] {
        let server = counting(Some(announced)).await;
        let channel = limited(&server.endpoint, None);
        let (code, message) = padded(&channel, padding).await;
        assert_eq!(
            server.requests.load(Ordering::SeqCst),
            usize::from(served),
            "a server announcing {announced} for a request of {weight}: {code:?} {message}"
        );
        assert_eq!(code == GrpcStatusCode::Ok, served, "{announced}: {message}");
    }
}
