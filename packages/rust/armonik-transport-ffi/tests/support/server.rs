//! The neighbouring crate's echo server, on threads of its own.
//!
//! The engine under test is two crates down, so what answers here owes it nothing - and it is the
//! very server the engine's own tests face, which is what keeps the two suites facing one peer.

use std::convert::Infallible;

use armonik_transport::reexports::hyper;
use armonik_transport::reexports::hyper_util::rt::{TokioExecutor, TokioIo};

use super::echo::answer;

/// A gRPC server on an ephemeral loopback port, on threads of its own.
///
/// Its own runtime, because the ABI under test owns the only other one and a test must be able to
/// shut that one down while the server is still answering.
pub struct TestServer {
    pub endpoint: String,
    _runtime: tokio::runtime::Runtime,
}

impl TestServer {
    pub fn start() -> Self {
        // One worker: the tests take turns at the ABI's single runtime, so no two servers ever
        // answer at once and a pool per test would be threads created only to be joined.
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
