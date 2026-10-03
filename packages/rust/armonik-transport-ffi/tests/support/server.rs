use std::convert::Infallible;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use armonik_transport::reexports::hyper;
use armonik_transport::reexports::hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

use super::echo::answer;

pub struct TestServer {
    pub endpoint: String,
    connections: Arc<AtomicUsize>,
    goodbyes: Arc<Mutex<Vec<bool>>>,
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
        let connections = Arc::new(AtomicUsize::new(0));
        let accepted = Arc::clone(&connections);
        let goodbyes = Arc::new(Mutex::new(Vec::new()));
        let ended = Arc::clone(&goodbyes);

        runtime.spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                accepted.fetch_add(1, Ordering::SeqCst);
                let ended = Arc::clone(&ended);
                tokio::spawn(async move {
                    let service = hyper::service::service_fn(|request| async {
                        Ok::<_, Infallible>(answer(request).await)
                    });
                    let mut recorded = Recorded {
                        stream,
                        read: Vec::new(),
                    };
                    let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                        .serve_connection(TokioIo::new(&mut recorded), service)
                        .await;
                    let said_goodbye = sent_goaway(&recorded.read);
                    ended
                        .lock()
                        .expect("one connection at a time")
                        .push(said_goodbye);
                });
            }
        });

        Self {
            endpoint: format!("http://{address}"),
            connections,
            goodbyes,
            _runtime: runtime,
        }
    }

    /// How many connections it accepted.
    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }

    /// For each connection that has ended, whether its client sent a GOAWAY on it: the frame
    /// RFC 9113 section 6.8 asks of an endpoint before it closes a connection.
    pub fn goodbyes(&self) -> Vec<bool> {
        self.goodbyes
            .lock()
            .expect("one connection at a time")
            .clone()
    }
}

/// A connection that keeps every byte its client sent, for the frames to be read once it ends.
struct Recorded {
    stream: TcpStream,
    read: Vec<u8>,
}

impl AsyncRead for Recorded {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let this = &mut *self;
        let polled = Pin::new(&mut this.stream).poll_read(cx, buf);
        this.read.extend_from_slice(&buf.filled()[before..]);
        polled
    }
}

impl AsyncWrite for Recorded {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

/// Walks the frames after the client's 24-byte preface: each a 9-byte header whose first three
/// bytes are its payload's length and whose fourth is its type, GOAWAY being 0x7.
fn sent_goaway(bytes: &[u8]) -> bool {
    let mut at = 24;
    while at + 9 <= bytes.len() {
        if bytes[at + 3] == 0x7 {
            return true;
        }
        let len = u32::from_be_bytes([0, bytes[at], bytes[at + 1], bytes[at + 2]]) as usize;
        at += 9 + len;
    }
    false
}
