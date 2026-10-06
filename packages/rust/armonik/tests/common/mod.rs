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

/// A client of `service`, served over HTTP/2 on a loopback port of its own, as the engine reaches a
/// server: through a connection, not in process.
#[allow(unused)]
pub(crate) fn client<S>(service: S) -> armonik::Client
where
    S: tower_service::Service<
            http::Request<hyper::body::Incoming>,
            Response = http::Response<tonic::body::Body>,
            Error = std::convert::Infallible,
        > + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    armonik::Client::with_channel(channel(service))
}

/// A channel to `service`, served as [`client`] serves it.
#[allow(unused)]
pub(crate) fn channel<S>(service: S) -> armonik::client::GrpcChannel
where
    S: tower_service::Service<
            http::Request<hyper::body::Incoming>,
            Response = http::Response<tonic::body::Body>,
            Error = std::convert::Infallible,
        > + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    listener
        .set_nonblocking(true)
        .expect("a listener tokio can take");
    let endpoint = format!("http://{}", listener.local_addr().expect("its address"));
    let listener = tokio::net::TcpListener::from_std(listener).expect("inside the test's runtime");
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let service = hyper_util::service::TowerToHyperService::new(service.clone());
            tokio::spawn(async move {
                let _ =
                    hyper::server::conn::http2::Builder::new(hyper_util::rt::TokioExecutor::new())
                        .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                        .await;
            });
        }
    });

    let transport = armonik_transport::http2::TransportConfig::new(
        endpoint.parse().expect("a loopback endpoint"),
    );
    armonik::client::GrpcChannel::new(
        armonik_transport::grpc::GrpcChannelConfig::new(transport),
        tokio::runtime::Handle::current(),
    )
    .expect("a channel")
}

/// Answers every call with its head and one empty message, then holds the response open.
#[allow(unused)]
#[derive(Clone)]
pub(crate) struct OneAnswerThenWait;

impl tower_service::Service<http::Request<hyper::body::Incoming>> for OneAnswerThenWait {
    type Response = http::Response<tonic::body::Body>;
    type Error = std::convert::Infallible;
    type Future = std::future::Ready<Result<Self::Response, Self::Error>>;

    fn poll_ready(
        &mut self,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: http::Request<hyper::body::Incoming>) -> Self::Future {
        let frames = async_stream::stream! {
            // Held, so that the request stays open as long as the response does.
            let _request = request;
            yield Ok::<_, tonic::Status>(hyper::body::Frame::data(hyper::body::Bytes::from_static(
                &[0; 5],
            )));
            std::future::pending::<()>().await;
        };
        let response = http::Response::builder()
            .header("content-type", "application/grpc")
            .body(tonic::body::Body::new(http_body_util::StreamBody::new(
                frames,
            )))
            .expect("a valid response");
        std::future::ready(Ok(response))
    }
}

/// A request stream that yields `first`, then waits for ever; `dropped` is cancelled once the
/// stream is dropped.
#[allow(unused)]
pub(crate) fn waiting_after<T>(
    first: T,
    dropped: &tokio_util::sync::CancellationToken,
) -> impl futures::Stream<Item = T> + Send + 'static
where
    T: Send + 'static,
{
    let guard = dropped.clone().drop_guard();
    async_stream::stream! {
        let _guard = guard;
        yield first;
        std::future::pending::<()>().await;
    }
}
