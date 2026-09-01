use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use armonik_transport::reexports::hyper;
use armonik_transport::reexports::hyper_util::rt::{TokioExecutor, TokioIo};
use armonik_transport::reexports::tonic::body::Body as TonicBody;
use armonik_transport::reexports::tonic::metadata::{MetadataMap, MetadataValue};
use armonik_transport::reexports::tonic::{Code, Request, Response, Status};
use bytes::Bytes;
use hyper::body::Incoming;

#[path = "../../../armonik-transport/tests/common/codec.rs"]
mod codec;

use codec::BytesCodec;
use tower_service::Service;

pub const ECHO: &str = "/armonik_transport_ffi.test.Echo/Echo";
pub const FAIL: &str = "/armonik_transport_ffi.test.Echo/Fail";
pub const SLOW: &str = "/armonik_transport_ffi.test.Echo/Slow";

type Answer = Pin<Box<dyn Future<Output = Result<Response<Bytes>, Status>> + Send>>;

#[derive(Clone, Copy)]
struct Handler(fn(Request<Bytes>) -> Answer);

impl Service<Request<Bytes>> for Handler {
    type Response = Response<Bytes>;
    type Error = Status;
    type Future = Answer;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Request<Bytes>) -> Self::Future {
        (self.0)(request)
    }
}

fn echo(request: Request<Bytes>) -> Answer {
    let echoed = request
        .metadata()
        .get("x-request")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| MetadataValue::try_from(value).ok());

    Box::pin(async move {
        let mut response = Response::new(request.into_inner());
        if let Some(echoed) = echoed {
            response.metadata_mut().insert("x-echoed", echoed);
        }
        Ok(response)
    })
}

fn fail(_request: Request<Bytes>) -> Answer {
    Box::pin(async move {
        Err(Status::with_metadata(
            Code::PermissionDenied,
            "not for you",
            MetadataMap::new(),
        ))
    })
}

fn slow(_request: Request<Bytes>) -> Answer {
    Box::pin(async move {
        tokio::time::sleep(Duration::from_secs(3600)).await;
        Ok(Response::new(Bytes::new()))
    })
}

async fn answer(request: hyper::Request<Incoming>) -> hyper::Response<TonicBody> {
    use armonik_transport::reexports::tonic::server::Grpc;

    let handler = match request.uri().path() {
        ECHO => echo,
        FAIL => fail,
        SLOW => slow,
        _ => return Status::unimplemented("no such method").into_http(),
    };

    Grpc::new(BytesCodec)
        .unary(&mut Handler(handler), request.map(TonicBody::new))
        .await
}

pub struct TestServer {
    pub endpoint: String,
    _runtime: tokio::runtime::Runtime,
}

impl TestServer {
    pub fn start() -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a runtime for the test server");

        let listener = runtime.block_on(async {
            tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind the test server")
        });
        let address = listener.local_addr().expect("the test server's address");

        runtime.spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let service = hyper::service::service_fn(|request| async {
                        Ok::<_, Infallible>(answer(request).await)
                    });
                    let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });

        Self {
            endpoint: format!("http://{address}"),
            _runtime: runtime,
        }
    }
}
