//! Against peers that read the wire frame by frame, per build and frames per write: the DATA a
//! cancelled upload still sends past the offset the client had written to when it cancelled, and
//! how long a server's PING waits for its acknowledgement, idle and behind an upload. Each under
//! two sets of flow-control windows, the largest HTTP/2 allows and Kestrel's defaults. Compiled by
//! both builds, the stock one at one frame per write only; `--nocapture` prints the figures, and
//! the PINGs, a measurement of up to about 80 s, run with `--ignored`.

mod common;

use std::time::{Duration, Instant};

use armonik_transport::grpc::{CallStartOptions, GrpcChannel, GrpcChannelConfig};
use armonik_transport::hooks;
use armonik_transport::http2::TransportConfig;
use bytes::Bytes;
use common::echo::{channel_with, COLLECT};
use http::Uri;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, Notify};

const DATA: u8 = 0x0;
const RST_STREAM: u8 = 0x3;
const SETTINGS: u8 = 0x4;
const PING: u8 = 0x6;
const WINDOW_UPDATE: u8 = 0x8;
const ACK: u8 = 0x1;
const PREFACE: usize = 24;
const HEAD: usize = 9;
/// The chunk of the streamed upload in ArmoniK.Api.Client.RustGrpcChannel.Benchmarks.
const CHUNK: usize = 64 * 1024;
const RESETS: usize = 10;
const PINGS: usize = 500;
/// How long the PINGs of one measurement go on, at most.
const PINGING: Duration = Duration::from_secs(20);
/// How long one PING may wait for its acknowledgement before the measurement fails.
const PATIENCE: Duration = Duration::from_secs(30);

/// Held by each test: `hooks::bytes_written` counts the writes of every connection in the process.
static ALONE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The flow-control windows a server announces.
#[derive(Clone, Copy)]
struct Windows {
    name: &'static str,
    stream: u32,
    connection: u32,
}

const LARGEST: Windows = Windows {
    name: "largest",
    stream: 0x7fff_ffff,
    connection: 0x7fff_ffff,
};

/// Kestrel's defaults, the server of ArmoniK.Api.Client.RustGrpcChannel.Benchmarks.
const KESTREL: Windows = Windows {
    name: "kestrel",
    stream: 768 * 1024,
    connection: 1024 * 1024,
};

/// The frames per write this build takes.
fn frame_counts() -> &'static [usize] {
    if cfg!(h2_batch) {
        &[1, 16]
    } else {
        &[1]
    }
}

struct Frame {
    kind: u8,
    flags: u8,
    stream: u32,
    payload: Vec<u8>,
}

/// The next frame, or none once the connection has ended.
async fn read_frame(socket: &mut (impl AsyncRead + Unpin)) -> Option<Frame> {
    let mut head = [0; HEAD];
    socket.read_exact(&mut head).await.ok()?;
    let length = u32::from_be_bytes([0, head[0], head[1], head[2]]) as usize;
    let mut payload = vec![0; length];
    socket.read_exact(&mut payload).await.ok()?;
    Some(Frame {
        kind: head[3],
        flags: head[4],
        stream: u32::from_be_bytes([head[5], head[6], head[7], head[8]]) & 0x7fff_ffff,
        payload,
    })
}

/// A window update of `increment` for `stream`, 0 for the connection.
fn window_update(stream: u32, increment: u32) -> Vec<u8> {
    let mut frame = vec![0, 0, 4, WINDOW_UPDATE, 0];
    frame.extend_from_slice(&stream.to_be_bytes());
    frame.extend_from_slice(&increment.to_be_bytes());
    frame
}

/// The server's SETTINGS, which set each stream's window, and the update that sets the
/// connection's, which starts at 65535.
fn hello(windows: Windows) -> Vec<u8> {
    let mut frames = vec![0, 0, 6, SETTINGS, 0, 0, 0, 0, 0, 0, 4];
    frames.extend_from_slice(&windows.stream.to_be_bytes());
    frames.extend(window_update(0, windows.connection - 65_535));
    frames
}

const SETTINGS_ACK: [u8; HEAD] = [0, 0, 0, SETTINGS, ACK, 0, 0, 0, 0];

fn channel_to(endpoint: &str, frames_per_write: usize) -> GrpcChannel {
    let uri = Uri::try_from(endpoint).expect("an endpoint");
    let mut transport = TransportConfig::new(uri);
    transport.http2.frames_per_write = frames_per_write;
    channel_with(GrpcChannelConfig::new(transport)).expect("a channel")
}

/// Where stream 1's DATA frames lay on the wire, from its first byte, and where its RST_STREAM
/// began.
struct Wire {
    data: Vec<(usize, usize)>,
    reset_at: usize,
}

/// A server that, at stream 1's first DATA frame, reads nothing more until `resume`, then reads to
/// stream 1's RST_STREAM and sends where its frames lay. It updates no window.
async fn stalling(
    windows: Windows,
    started: oneshot::Sender<()>,
    resume: std::sync::Arc<Notify>,
    wire: oneshot::Sender<Wire>,
) -> String {
    let socket = tokio::net::TcpSocket::new_v4().expect("a socket");
    socket
        .set_recv_buffer_size(4096)
        .expect("a small receive buffer");
    socket.bind("127.0.0.1:0".parse().unwrap()).expect("bind");
    let endpoint = format!("http://{}", socket.local_addr().expect("an address"));
    let listener = socket.listen(1).expect("listen");
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("the client connects");
        let mut preface = [0; PREFACE];
        socket.read_exact(&mut preface).await.expect("the preface");
        socket
            .write_all(&hello(windows))
            .await
            .expect("the settings");
        let mut offset = PREFACE;
        let mut data = Vec::new();
        let mut started = Some(started);
        while let Some(frame) = read_frame(&mut socket).await {
            let at = offset;
            offset += HEAD + frame.payload.len();
            match frame.kind {
                SETTINGS if frame.flags & ACK == 0 => {
                    let _ = socket.write_all(&SETTINGS_ACK).await;
                }
                DATA if frame.stream == 1 => {
                    data.push((at, offset));
                    if let Some(started) = started.take() {
                        let _ = started.send(());
                        resume.notified().await;
                    }
                }
                RST_STREAM if frame.stream == 1 => {
                    let _ = wire.send(Wire { data, reset_at: at });
                    return;
                }
                _ => {}
            }
        }
    });
    endpoint
}

/// What of a cancelled upload went on the wire after its cancel.
struct Tail {
    /// The place on the wire the client had written to when it cancelled.
    cancelled_at: usize,
    /// The DATA frames that end past it.
    frames: usize,
    /// Their bytes past it.
    bytes: usize,
}

/// Until no connection of the process has written for `quiet`: the last count of bytes written.
async fn writes_stop(quiet: Duration) -> usize {
    let mut written = hooks::bytes_written();
    loop {
        tokio::time::sleep(quiet).await;
        let now = hooks::bytes_written();
        if now == written {
            return written;
        }
        written = now;
    }
}

/// A streamed upload of `CHUNK`s, cancelled once its connection's writes have stopped against a
/// peer that reads nothing, whether on a full socket or on an exhausted window.
async fn after_a_cancel(frames_per_write: usize, windows: Windows) -> Tail {
    let (started, has_started) = oneshot::channel();
    let resume = std::sync::Arc::new(Notify::new());
    let (wire, has_wire) = oneshot::channel();
    let endpoint = stalling(windows, started, resume.clone(), wire).await;
    // Past what the previous measurement's connection still writes as it closes.
    let base = writes_stop(Duration::from_millis(100)).await;
    let channel = channel_to(&endpoint, frames_per_write);
    let (mut send, recv, control) = channel
        .start_call(CallStartOptions::new(COLLECT))
        .expect("the call starts")
        .split();
    let sending = tokio::spawn(async move {
        let chunk = Bytes::from(vec![0x5a; CHUNK]);
        while send.send_message(chunk.clone()).await.is_ok() {}
    });
    tokio::time::timeout(Duration::from_secs(5), has_started)
        .await
        .expect("the upload reaches the server")
        .expect("the server");

    let written = writes_stop(Duration::from_millis(300)).await;
    control.cancel();
    // Time for the cancel to reach the connection before the peer reads again.
    tokio::time::sleep(Duration::from_millis(200)).await;
    resume.notify_one();
    let wire = tokio::time::timeout(Duration::from_secs(10), has_wire)
        .await
        .expect("the reset reaches the server")
        .expect("the server");
    sending.abort();
    drop(recv);

    let cancelled_at = written - base;
    assert!(wire.reset_at >= cancelled_at, "a reset before the cancel");
    let after: Vec<_> = wire
        .data
        .iter()
        .filter(|(_, end)| *end > cancelled_at)
        .collect();
    Tail {
        cancelled_at,
        frames: after.len(),
        bytes: after
            .iter()
            .map(|(start, end)| end - (*start).max(cancelled_at))
            .sum(),
    }
}

/// A cancelled upload sends at most one DATA frame past the offset its cancel found on the wire at
/// one frame per write, in either build; above one, what it sends is printed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn data_after_a_cancel() {
    let _alone = ALONE.lock().await;
    for windows in [LARGEST, KESTREL] {
        for &frames in frame_counts() {
            let mut tails = Vec::new();
            for _ in 0..RESETS {
                tails.push(after_a_cancel(frames, windows).await);
            }
            for tail in &tails {
                println!(
                    "h2_batch={} windows={} frames_per_write={frames} cancelled_at={} after_cancel_frames={} after_cancel_bytes={}",
                    cfg!(h2_batch),
                    windows.name,
                    tail.cancelled_at,
                    tail.frames,
                    tail.bytes,
                );
            }
            for tail in &tails {
                if frames == 1 {
                    assert!(
                        tail.frames <= 1,
                        "{} DATA frames after the cancel at one frame per write",
                        tail.frames
                    );
                }
            }
        }
    }
}

/// A server that sends PINGs one at a time, from stream 1's first DATA frame on, for `PINGS` or
/// `PINGING`, and sends how long each waited for its acknowledgement: from its write to the
/// server's read of the acknowledgement, behind whatever the client sent before it. It reads
/// everything else, dropping it, and gives back the windows as it reads, half of one at a time.
async fn pinging(windows: Windows, delays: oneshot::Sender<Vec<Duration>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let endpoint = format!("http://{}", listener.local_addr().expect("an address"));
    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("the client connects");
        socket.set_nodelay(true).expect("no delay");
        let (mut reading, writing) = socket.into_split();
        let mut preface = [0; PREFACE];
        reading.read_exact(&mut preface).await.expect("the preface");
        let writing = std::sync::Arc::new(tokio::sync::Mutex::new(writing));
        writing
            .lock()
            .await
            .write_all(&hello(windows))
            .await
            .expect("the settings");
        let flowing = std::sync::Arc::new(Notify::new());
        let (acknowledged, mut acknowledgements) = mpsc::unbounded_channel();

        let pinger = {
            let (writing, flowing) = (writing.clone(), flowing.clone());
            tokio::spawn(async move {
                flowing.notified().await;
                let begun = Instant::now();
                let mut measured = Vec::with_capacity(PINGS);
                for ping in 0..PINGS as u64 {
                    if begun.elapsed() > PINGING {
                        break;
                    }
                    let mut frame = vec![0, 0, 8, PING, 0, 0, 0, 0, 0];
                    frame.extend_from_slice(&ping.to_be_bytes());
                    let mut writing = writing.lock().await;
                    let sent = Instant::now();
                    writing.write_all(&frame).await.expect("a PING");
                    drop(writing);
                    let at: Instant = tokio::time::timeout(PATIENCE, acknowledgements.recv())
                        .await
                        .unwrap_or_else(|_| panic!("a PING unacknowledged for {PATIENCE:?}"))
                        .expect("the reader");
                    measured.push(at - sent);
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
                let _ = delays.send(measured);
            })
        };

        let mut flowing = Some(flowing);
        let mut unreturned = 0_u32;
        while let Some(frame) = read_frame(&mut reading).await {
            match frame.kind {
                SETTINGS if frame.flags & ACK == 0 => {
                    let _ = writing.lock().await.write_all(&SETTINGS_ACK).await;
                }
                PING if frame.flags & ACK != 0 => {
                    let _ = acknowledged.send(Instant::now());
                }
                DATA if frame.stream == 1 => {
                    if let Some(flowing) = flowing.take() {
                        flowing.notify_one();
                    }
                    unreturned += frame.payload.len() as u32;
                    if unreturned >= windows.stream / 2 {
                        let mut frames = window_update(0, unreturned);
                        frames.extend(window_update(1, unreturned));
                        let _ = writing.lock().await.write_all(&frames).await;
                        unreturned = 0;
                    }
                }
                _ => {}
            }
            if pinger.is_finished() {
                return;
            }
        }
    });
    endpoint
}

/// How long the server's PINGs waited for their acknowledgements, sorted, over a connection whose
/// one call sends one byte and then nothing when `uploading` is false, and a stream of `CHUNK`s
/// for as long as the PINGs last when it is true.
async fn ping_delays(frames_per_write: usize, windows: Windows, uploading: bool) -> Vec<Duration> {
    let (delays, has_delays) = oneshot::channel();
    let endpoint = pinging(windows, delays).await;
    let channel = channel_to(&endpoint, frames_per_write);
    let (mut send, recv, control) = channel
        .start_call(CallStartOptions::new(COLLECT))
        .expect("the call starts")
        .split();
    let sending = tokio::spawn(async move {
        if uploading {
            let chunk = Bytes::from(vec![0x5a; CHUNK]);
            while send.send_message(chunk.clone()).await.is_ok() {}
        } else {
            let _ = send.send_message(Bytes::from_static(b"x")).await;
            std::future::pending::<()>().await;
        }
    });
    let mut delays = tokio::time::timeout(PINGING + 2 * PATIENCE, has_delays)
        .await
        .expect("the PINGs end")
        .expect("every PING is acknowledged");
    control.cancel();
    sending.abort();
    drop(recv);
    delays.sort_unstable();
    delays
}

/// The value below which `fraction` of the sorted `delays` fall, in microseconds.
fn percentile(delays: &[Duration], fraction: f64) -> u128 {
    let at = (delays.len() as f64 * fraction).ceil() as usize;
    delays[at.saturating_sub(1)].as_micros()
}

/// Every PING is acknowledged, behind an upload too; how long each waited is printed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "a measurement of up to about 80 s"]
async fn a_ping_behind_an_upload() {
    let _alone = ALONE.lock().await;
    for windows in [LARGEST, KESTREL] {
        for &frames in frame_counts() {
            for uploading in [false, true] {
                let delays = ping_delays(frames, windows, uploading).await;
                println!(
                    "h2_batch={} windows={} frames_per_write={frames} uploading={uploading} pings={} ping_p50_us={} ping_p99_us={} ping_max_us={}",
                    cfg!(h2_batch),
                    windows.name,
                    delays.len(),
                    percentile(&delays, 0.50),
                    percentile(&delays, 0.99),
                    percentile(&delays, 1.0),
                );
            }
        }
    }
}
