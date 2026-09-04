#![allow(dead_code)]

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use armonik_transport::reexports::hyper;
use armonik_transport::reexports::tonic::body::Body;
use armonik_transport::reexports::tonic::server::NamedService;
use armonik_transport::reexports::tonic::{Request, Response, Status};
use bytes::Bytes;
use tower_service::Service;

mod codec;
pub mod echo;

use codec::BytesCodec;

pub const METHOD_PATH: &str = "/armonik_transport.test.Slow/Call";

pub const REPLY: &[u8] = b"served";

#[derive(Clone)]
pub struct SlowService {
    delay: Duration,
}

impl SlowService {
    pub fn new(delay: Duration) -> Self {
        Self { delay }
    }
}

impl NamedService for SlowService {
    const NAME: &'static str = "armonik_transport.test.Slow";
}

impl Service<Request<Bytes>> for SlowService {
    type Response = Response<Bytes>;
    type Error = Status;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _request: Request<Bytes>) -> Self::Future {
        let delay = self.delay;
        Box::pin(async move {
            tokio::time::sleep(delay).await;
            Ok(Response::new(Bytes::from_static(REPLY)))
        })
    }
}

impl Service<hyper::Request<Body>> for SlowService {
    type Response = hyper::Response<Body>;
    type Error = std::convert::Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: hyper::Request<Body>) -> Self::Future {
        let mut handler = self.clone();
        Box::pin(async move {
            Ok(
                armonik_transport::reexports::tonic::server::Grpc::new(BytesCodec)
                    .unary(&mut handler, request)
                    .await,
            )
        })
    }
}

pub async fn serve(service: SlowService) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the test server");
    let address = listener.local_addr().expect("the test server's address");

    tokio::spawn(async move {
        let incoming =
            armonik_transport::reexports::tonic::transport::server::TcpIncoming::from(listener);
        armonik_transport::reexports::tonic::transport::Server::builder()
            .add_service(service)
            .serve_with_incoming(incoming)
            .await
            .expect("serve the test service");
    });

    format!("http://{address}")
}

pub async fn call(
    channel: armonik_transport::reexports::tonic::transport::Channel,
) -> Result<Bytes, Status> {
    let path = armonik_transport::reexports::tonic::codegen::http::uri::PathAndQuery::try_from(
        METHOD_PATH,
    )
    .expect("a valid method path");

    let mut grpc = armonik_transport::reexports::tonic::client::Grpc::new(channel);
    grpc.ready()
        .await
        .map_err(|error| Status::unknown(format!("the channel was not ready: {error}")))?;

    let response = grpc
        .unary(Request::new(Bytes::from_static(b"ping")), path, BytesCodec)
        .await?;
    Ok(response.into_inner())
}

#[allow(clippy::field_reassign_with_default)]
pub fn config(
    endpoint: &str,
    set: impl FnOnce(&mut armonik_transport::ClientConfigArgs),
) -> armonik_transport::ClientConfig {
    let mut args = armonik_transport::ClientConfigArgs::default();
    args.endpoint = endpoint.to_owned();
    args.allow_unsafe_connection = true;
    set(&mut args);
    armonik_transport::ClientConfig::from_config_args(args)
        .expect("the configuration should be valid")
}
