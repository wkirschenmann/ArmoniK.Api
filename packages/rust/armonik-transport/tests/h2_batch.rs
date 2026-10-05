//! The build against h2-batch's patch: a request's DATA frames gathered into fewer writes, as
//! many per queued part as the connection is set to. Compiled by that build only, through
//! `packages/rust/patches/h2-batch/build.sh`.

#![cfg(h2_batch)]

mod common;

use armonik_transport::grpc::{CallStartOptions, GrpcChannelConfig, GrpcStatusCode};
use armonik_transport::hooks;
use armonik_transport::http2::{FixedWindows, ReceiveWindows, TransportConfig};
use bytes::Bytes;
use common::echo::{channel_with, unary, TestServer, ECHO};
use http::Uri;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const MESSAGE: usize = 4 * 1024 * 1024;

/// Held by each test: `hooks::writes` counts the writes of every connection in the process.
static ALONE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
/// More than loopback's socket buffers hold, so that the client's writes block.
const BLOCKING: usize = 32 * 1024 * 1024;

/// The writes of one request of `MESSAGE` bytes, past a first call that settles the session. Its
/// bytes count up, so that a part split into frames comes back whole and in order.
async fn writes_of_a_large_request(frames_per_write: usize) -> usize {
    let server = TestServer::start().await;
    let uri = Uri::try_from(server.endpoint.as_str()).expect("the test server's endpoint");
    let mut transport = TransportConfig::new(uri);
    transport.http2.frames_per_write = frames_per_write;
    transport.http2.receive_windows = ReceiveWindows::Fixed(FixedWindows {
        stream: 16 * 1024 * 1024,
        connection: 16 * 1024 * 1024,
    });
    let mut config = GrpcChannelConfig::new(transport);
    config.max_recv_message_size = 2 * MESSAGE;
    let channel = channel_with(config).expect("a plain endpoint");

    let (_, _, status) = unary(
        &channel,
        CallStartOptions::new(ECHO),
        Bytes::from_static(b"x"),
    )
    .await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");

    let sent: Bytes = (0..MESSAGE).map(|i| (i % 251) as u8).collect();
    let before = hooks::writes();
    let (_, messages, status) = unary(&channel, CallStartOptions::new(ECHO), sent.clone()).await;
    let writes = hooks::writes() - before;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert!(
        messages[0] == sent,
        "the echo differs at {frames_per_write} frames"
    );
    writes
}

/// At 16 frames a part, a request of 4 MiB goes out in fewer than a quarter of the writes it
/// takes at one.
#[tokio::test]
async fn more_frames_per_write_is_fewer_writes() {
    let _alone = ALONE.lock().await;
    let one = writes_of_a_large_request(1).await;
    let sixteen = writes_of_a_large_request(16).await;
    assert!(
        4 * sixteen < one,
        "{sixteen} writes at 16 frames, {one} at 1"
    );
}

const DATA: u8 = 0x0;
const HEADERS: u8 = 0x1;
const RST_STREAM: u8 = 0x3;
const SETTINGS: u8 = 0x4;
const WINDOW_UPDATE: u8 = 0x8;
const ACK: u8 = 0x1;
const END_STREAM: u8 = 0x1;
const END_HEADERS: u8 = 0x4;

/// A server that lets the client send as far ahead as HTTP/2 allows and, at stream 1's first DATA
/// frame, stops reading and resets it, so that the client takes the reset and releases the stream
/// while a part of it is still half written. Every later request it answers with an OK.
async fn resetting() -> String {
    let socket = tokio::net::TcpSocket::new_v4().expect("a socket");
    socket
        .set_recv_buffer_size(4096)
        .expect("a small receive buffer");
    socket.bind("127.0.0.1:0".parse().unwrap()).expect("bind");
    let endpoint = format!("http://{}", socket.local_addr().expect("an address"));
    let listener = socket.listen(1).expect("listen");
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("the client connects");
        let mut preface = [0; 24];
        socket.read_exact(&mut preface).await.expect("the preface");
        // The largest window, for each stream and for the connection, whose own starts at 65535.
        let window: u32 = 0x7fff_ffff;
        let mut hello = vec![0, 0, 6, SETTINGS, 0, 0, 0, 0, 0, 0, 4];
        hello.extend_from_slice(&window.to_be_bytes());
        hello.extend_from_slice(&[0, 0, 4, WINDOW_UPDATE, 0, 0, 0, 0, 0]);
        hello.extend_from_slice(&(window - 65_535).to_be_bytes());
        socket
            .write_all(&hello)
            .await
            .expect("the server's settings");

        let mut reset = false;
        loop {
            let mut head = [0; 9];
            if socket.read_exact(&mut head).await.is_err() {
                return;
            }
            let length = u32::from_be_bytes([0, head[0], head[1], head[2]]) as usize;
            let (kind, flags) = (head[3], head[4]);
            let stream = u32::from_be_bytes([head[5], head[6], head[7], head[8]]) & 0x7fff_ffff;
            let mut payload = vec![0; length];
            if socket.read_exact(&mut payload).await.is_err() {
                return;
            }
            match kind {
                SETTINGS if flags & ACK == 0 => {
                    let _ = socket
                        .write_all(&[0, 0, 0, SETTINGS, ACK, 0, 0, 0, 0])
                        .await;
                }
                DATA if stream == 1 && !reset => {
                    reset = true;
                    // Unread, the client's writes block with a part of stream 1 half written.
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    // CANCEL, on stream 1.
                    let _ = socket
                        .write_all(&[0, 0, 4, RST_STREAM, 0, 0, 0, 0, 1, 0, 0, 0, 8])
                        .await;
                    // Time for the client to take the reset and release the stream.
                    tokio::time::sleep(Duration::from_millis(300)).await;
                }
                DATA if stream > 1 && flags & END_STREAM != 0 => {
                    let _ = socket.write_all(&trailers_only_ok(stream)).await;
                }
                _ => {}
            }
        }
    });
    endpoint
}

/// A HEADERS frame that ends `stream` with `grpc-status: 0` and no message, its fields encoded
/// with HPACK's static table and literals only.
fn trailers_only_ok(stream: u32) -> Vec<u8> {
    let mut block = vec![0x88]; // :status 200
    block.extend_from_slice(&[0x0f, 0x10, 16]); // content-type, static index 31, not indexed
    block.extend_from_slice(b"application/grpc");
    block.extend_from_slice(&[0x00, 11]);
    block.extend_from_slice(b"grpc-status");
    block.extend_from_slice(&[1, b'0']);
    let mut frame = (block.len() as u32).to_be_bytes()[1..].to_vec();
    frame.extend_from_slice(&[HEADERS, END_STREAM | END_HEADERS]);
    frame.extend_from_slice(&stream.to_be_bytes());
    frame.extend_from_slice(&block);
    frame
}

/// A peer that resets a request while a part of it is half written leaves the connection usable:
/// the rest of the part is dropped, even once the stream is released.
#[tokio::test]
async fn a_request_reset_mid_write_leaves_the_connection_usable() {
    let _alone = ALONE.lock().await;
    for frames in [1, 16] {
        let uri = Uri::try_from(resetting().await.as_str()).expect("an endpoint");
        let mut transport = TransportConfig::new(uri);
        transport.http2.frames_per_write = frames;
        let channel = channel_with(GrpcChannelConfig::new(transport)).expect("a channel");

        let (_, _, status) = tokio::time::timeout(
            Duration::from_secs(5),
            unary(
                &channel,
                CallStartOptions::new(ECHO),
                Bytes::from(vec![0x5a; BLOCKING]),
            ),
        )
        .await
        .expect("the reset call ends");
        assert_ne!(status.code, GrpcStatusCode::Ok, "{status}");

        let (_, _, status) = tokio::time::timeout(
            Duration::from_secs(5),
            unary(
                &channel,
                CallStartOptions::new(ECHO),
                Bytes::from_static(b"x"),
            ),
        )
        .await
        .unwrap_or_else(|_| panic!("no answer on the connection at {frames} frames"));
        assert_eq!(
            status.code,
            GrpcStatusCode::Ok,
            "at {frames} frames: {status}"
        );
    }
}
