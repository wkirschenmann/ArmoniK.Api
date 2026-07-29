//! A minimal, hand-rolled gRPC service for the integration tests in this crate.
//!
//! `armonik-transport` deliberately depends on no generated proto types (that is the whole point of
//! the split from `armonik`), so its own tests cannot spin up a service via `tonic-build`-generated
//! server traits either — doing so would mean depending on `armonik` from `armonik-transport`,
//! which `armonik` itself depends on, a cycle Cargo will not allow.
//!
//! [`RawEchoService`] sidesteps that entirely: it is the server-side counterpart of the raw-bytes
//! codec `armonik-transport-ffi` uses on the client side, always answering a unary call with a
//! fixed, caller-chosen payload regardless of the method path. That is enough to prove a real gRPC
//! round trip went through the connector stack under test (TCP, the proxy tunnel, TLS) end to end,
//! without needing to know anything about ArmoniK's actual messages.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use armonik_transport::reexports::http;
use armonik_transport::reexports::tonic::body::Body;
use armonik_transport::reexports::tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use armonik_transport::reexports::tonic::server::{Grpc, NamedService};
use armonik_transport::reexports::tonic::{Request, Response, Status};
use bytes::{Buf, BufMut, Bytes};
use tower_service::Service;

#[allow(unused)]
pub(crate) async fn unary_rpc_impl<Response>(
    duration: Option<tokio::time::Duration>,
    failure: Option<tonic::Status>,
    response: impl FnOnce() -> Result<Response, tonic::Status>,
) -> Result<Response, tonic::Status> {
    if let Some(duration) = duration {
        tokio::time::sleep(duration).await;
    }

    if let Some(failure) = failure {
        Err(failure)
    } else {
        response()
    }
}

/// The raw-bytes codec, mirroring `armonik-transport-ffi`'s `BytesCodec` on the server side.
#[derive(Debug, Clone, Copy, Default)]
struct BytesCodec;

impl Codec for BytesCodec {
    type Encode = Bytes;
    type Decode = Bytes;
    type Encoder = Self;
    type Decoder = Self;

    fn encoder(&mut self) -> Self::Encoder {
        *self
    }

    fn decoder(&mut self) -> Self::Decoder {
        *self
    }
}

impl Encoder for BytesCodec {
    type Item = Bytes;
    type Error = Status;

    fn encode(&mut self, item: Self::Item, dst: &mut EncodeBuf<'_>) -> Result<(), Self::Error> {
        dst.reserve(item.len());
        dst.put_slice(&item);
        Ok(())
    }
}

impl Decoder for BytesCodec {
    type Item = Bytes;
    type Error = Status;

    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Self::Error> {
        let len = src.remaining();
        Ok(Some(src.copy_to_bytes(len)))
    }
}

/// A unary handler that always answers with the same, fixed payload.
#[derive(Clone)]
struct FixedResponse(Bytes);

impl Service<Request<Bytes>> for FixedResponse {
    type Response = Response<Bytes>;
    type Error = Status;
    type Future = std::future::Ready<Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _request: Request<Bytes>) -> Self::Future {
        std::future::ready(Ok(Response::new(self.0.clone())))
    }
}

/// A `tonic`-servable, single-method gRPC service that answers every unary call with the same
/// fixed payload, whatever the method path.
#[derive(Debug, Clone)]
pub(crate) struct RawEchoService {
    response: Bytes,
}

impl RawEchoService {
    pub(crate) fn new(response: impl Into<Bytes>) -> Self {
        Self {
            response: response.into(),
        }
    }
}

impl NamedService for RawEchoService {
    // Only used for `tonic::transport::Server`'s routing table; these tests call through
    // `armonik_transport::connect`, which never inspects it.
    const NAME: &'static str = "armonik_transport.test.RawEcho";
}

impl Service<http::Request<Body>> for RawEchoService {
    type Response = http::Response<Body>;
    type Error = std::convert::Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: http::Request<Body>) -> Self::Future {
        let handler = FixedResponse(self.response.clone());
        Box::pin(async move { Ok(Grpc::new(BytesCodec).unary(handler, request).await) })
    }
}

/// Serve `service` on an ephemeral loopback port, returning the address it is listening on.
pub(crate) async fn spawn_raw_server(response: impl Into<Bytes>) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind server");
    let address = listener.local_addr().expect("server address");
    let service = RawEchoService::new(response);

    tokio::spawn(async move {
        let incoming =
            armonik_transport::reexports::tokio_stream::wrappers::TcpListenerStream::new(listener);
        armonik_transport::reexports::tonic::transport::Server::builder()
            .add_service(service)
            .serve_with_incoming(incoming)
            .await
            .expect("serve");
    });

    address
}

/// Make a raw unary call over an established channel, without any generated proto types — the
/// client-side counterpart of [`RawEchoService`], letting tests round-trip a message through a
/// real, connected channel.
pub(crate) async fn call_raw_unary(
    channel: armonik_transport::reexports::tonic::transport::Channel,
    payload: impl Into<Bytes>,
) -> Result<Bytes, Status> {
    let path = armonik_transport::reexports::http::uri::PathAndQuery::try_from(
        "/armonik_transport.test.RawEcho/Call",
    )
    .expect("a valid path");
    let mut grpc = armonik_transport::reexports::tonic::client::Grpc::new(channel);
    // Generated client code always does this before a call; `Grpc::unary` does not do it
    // implicitly, and skipping it trips `tower::Buffer`'s "send_item called without first calling
    // poll_reserve" assertion.
    grpc.ready()
        .await
        .map_err(|error| Status::unavailable(format!("the channel was not ready: {error}")))?;
    let response = grpc
        .unary(Request::new(payload.into()), path, BytesCodec)
        .await?;
    Ok(response.into_inner())
}
