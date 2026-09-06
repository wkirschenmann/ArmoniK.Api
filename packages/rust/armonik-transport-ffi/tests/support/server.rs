use std::convert::Infallible;

use armonik_transport::reexports::hyper;
use armonik_transport::reexports::hyper_util::rt::{TokioExecutor, TokioIo};

use super::echo::answer;

pub struct TestServer {
    pub endpoint: String,
    _runtime: tokio::runtime::Runtime,
}

impl TestServer {
    pub fn start() -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
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
