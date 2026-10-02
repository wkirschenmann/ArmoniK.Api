//! A server whose HTTP/2 layer turns streams away before any application sees them, as a peer
//! that is shutting down or over its stream limit does.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http::header::HeaderMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::echo::loopback;

/// How the first streams are turned away.
#[derive(Clone, Copy, Debug)]
pub enum Refusal {
    /// RST_STREAM with REFUSED_STREAM, on a connection that goes on serving.
    RefusedStream,
    /// RST_STREAM with INTERNAL_ERROR, a reset that does not say the stream went unprocessed.
    InternalError,
    /// A GOAWAY naming no stream as processed, the connection then left for the client to close.
    GoAway,
    /// A GOAWAY naming the stream as processed, then the connection closed under it.
    GoAwayProcessed,
}

/// Turns the first `refused` streams away as its [`Refusal`] says, once it has read all they
/// send, and answers every other with the message it was sent.
pub struct Refuser {
    pub endpoint: String,
    seen: Arc<Mutex<Vec<Option<String>>>>,
}

impl Refuser {
    pub async fn start(refusal: Refusal, refused: usize) -> Self {
        let (listener, endpoint) = loopback().await;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let left = Arc::new(AtomicUsize::new(refused));
        let recorded = Arc::clone(&seen);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let seen = Arc::clone(&recorded);
                let left = Arc::clone(&left);
                tokio::spawn(async move {
                    match refusal {
                        Refusal::RefusedStream => {
                            refuse_streams(stream, seen, left, h2::Reason::REFUSED_STREAM).await
                        }
                        Refusal::InternalError => {
                            refuse_streams(stream, seen, left, h2::Reason::INTERNAL_ERROR).await
                        }
                        Refusal::GoAway if take(&left) => go_away(stream, seen, 0).await,
                        Refusal::GoAwayProcessed if take(&left) => go_away(stream, seen, 1).await,
                        Refusal::GoAway | Refusal::GoAwayProcessed => {
                            refuse_streams(stream, seen, left, h2::Reason::NO_ERROR).await
                        }
                    }
                });
            }
        });
        Self { endpoint, seen }
    }

    /// The `grpc-previous-rpc-attempts` of each stream it was sent, in order; a stream a GOAWAY
    /// turned away is never read, and counts as `None`.
    pub fn seen(&self) -> Vec<Option<String>> {
        self.seen.lock().expect("the record").clone()
    }
}

fn take(left: &AtomicUsize) -> bool {
    left.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
        left.checked_sub(1)
    })
    .is_ok()
}

/// Through h2's server, which resets a stream with the reason it is given.
async fn refuse_streams(
    stream: TcpStream,
    seen: Arc<Mutex<Vec<Option<String>>>>,
    left: Arc<AtomicUsize>,
    reason: h2::Reason,
) {
    let Ok(mut connection) = h2::server::handshake(stream).await else {
        return;
    };
    while let Some(Ok((request, mut respond))) = connection.accept().await {
        seen.lock().expect("the record").push(
            request
                .headers()
                .get("grpc-previous-rpc-attempts")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
        );
        let refused = take(&left);
        tokio::spawn(async move {
            let mut body = request.into_body();
            let mut sent = Vec::new();
            while let Some(Ok(chunk)) = body.data().await {
                let _ = body.flow_control().release_capacity(chunk.len());
                sent.extend_from_slice(&chunk);
            }
            if refused {
                respond.send_reset(reason);
                return;
            }
            let head = http::Response::builder()
                .header("content-type", "application/grpc")
                .body(())
                .expect("a head");
            let Ok(mut answer) = respond.send_response(head, false) else {
                return;
            };
            // The request's own framing, so the answer is the message it was sent.
            let _ = answer.send_data(Bytes::from(sent), false);
            let mut trailers = HeaderMap::new();
            trailers.insert("grpc-status", http::HeaderValue::from_static("0"));
            let _ = answer.send_trailers(trailers);
        });
    }
}

/// By hand, since h2's own GOAWAY names the last stream it received as processed: an empty
/// SETTINGS, then a GOAWAY naming `last` once the first stream has ended its request. Past it,
/// the connection is left for the client to close; at it, closed.
async fn go_away(mut stream: TcpStream, seen: Arc<Mutex<Vec<Option<String>>>>, last: u8) {
    const SETTINGS: u8 = 4;
    const DATA: u8 = 0;
    const HEADERS: u8 = 1;
    const PING: u8 = 6;
    // The same bit: ACK on SETTINGS and PING, END_STREAM on DATA and HEADERS.
    const ACK: u8 = 1;
    const END_STREAM: u8 = 1;

    let mut preface = [0u8; 24];
    if stream.read_exact(&mut preface).await.is_err() {
        return;
    }
    if stream
        .write_all(&[0, 0, 0, SETTINGS, 0, 0, 0, 0, 0])
        .await
        .is_err()
    {
        return;
    }
    loop {
        let mut header = [0u8; 9];
        if stream.read_exact(&mut header).await.is_err() {
            return;
        }
        let length = u32::from_be_bytes([0, header[0], header[1], header[2]]) as usize;
        // No client of these tests sends more than the default SETTINGS_MAX_FRAME_SIZE.
        if length > 16_384 {
            return;
        }
        let mut payload = vec![0; length];
        if stream.read_exact(&mut payload).await.is_err() {
            return;
        }
        let (kind, flags) = (header[3], header[4]);
        if kind == HEADERS {
            seen.lock().expect("the record").push(None);
        }
        let reply: Vec<u8> = match kind {
            SETTINGS if flags & ACK == 0 => vec![0, 0, 0, SETTINGS, ACK, 0, 0, 0, 0],
            PING if flags & ACK == 0 => [&[0, 0, 8, PING, ACK, 0, 0, 0, 0][..], &payload].concat(),
            DATA | HEADERS if flags & END_STREAM != 0 => break,
            _ => continue,
        };
        if stream.write_all(&reply).await.is_err() {
            return;
        }
    }
    // GOAWAY, NO_ERROR.
    if stream
        .write_all(&[0, 0, 8, 7, 0, 0, 0, 0, 0, 0, 0, 0, last, 0, 0, 0, 0])
        .await
        .is_err()
        || last > 0
    {
        return;
    }
    // Read on until the client closes, as closing first could reset the connection before the
    // client reads the frame.
    let mut drained = [0u8; 1024];
    while stream.read(&mut drained).await.is_ok_and(|read| read > 0) {}
}
