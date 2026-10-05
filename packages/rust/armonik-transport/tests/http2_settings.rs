//! What the HTTP/2 session announces and how it watches its peer, read off the wire by a server
//! that speaks HTTP/2 by hand.

mod common;

use std::time::Duration;

use armonik_transport::grpc::{CallStartOptions, GrpcChannelConfig, GrpcStatusCode};
use armonik_transport::http2::{Http2Config, TransportConfig};
use bytes::Bytes;
use common::echo::{channel_with, loopback, unary, ECHO};
use http::Uri;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;

const SETTINGS: u8 = 0x4;
const PING: u8 = 0x6;
const WINDOW_UPDATE: u8 = 0x8;
const HEADERS: u8 = 0x1;
const ACK: u8 = 0x1;
const SETTINGS_INITIAL_WINDOW_SIZE: u16 = 0x4;
/// RFC 9113's initial window, which a WINDOW_UPDATE on stream 0 grows the connection's from.
const INITIAL_WINDOW: u32 = 65_535;
const EMPTY_SETTINGS: [u8; 9] = [0, 0, 0, SETTINGS, 0, 0, 0, 0, 0];

/// What the client announced before its first request.
#[derive(Debug, Default)]
struct Announced {
    stream_window: Option<u32>,
    connection_window: Option<u32>,
}

/// A server that reads the client up to its first HEADERS, sends empty SETTINGS, and then only
/// reads: it answers no request and acknowledges no PING. `pinged` hears of the first PING.
async fn listening(heard: oneshot::Sender<Announced>, pinged: oneshot::Sender<()>) -> String {
    let (listener, endpoint) = loopback().await;
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("the client connects");
        let mut preface = [0; 24];
        socket.read_exact(&mut preface).await.expect("the preface");

        let mut announced = Announced::default();
        let mut heard = Some(heard);
        let mut pinged = Some(pinged);
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
                    for entry in payload.chunks_exact(6) {
                        let id = u16::from_be_bytes([entry[0], entry[1]]);
                        let value = u32::from_be_bytes([entry[2], entry[3], entry[4], entry[5]]);
                        if id == SETTINGS_INITIAL_WINDOW_SIZE {
                            announced.stream_window = Some(value);
                        }
                    }
                }
                WINDOW_UPDATE if stream == 0 => {
                    let increment =
                        u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]])
                            & 0x7fff_ffff;
                    let window = announced.connection_window.get_or_insert(INITIAL_WINDOW);
                    *window += increment;
                }
                HEADERS => {
                    if let Some(heard) = heard.take() {
                        let _ = heard.send(std::mem::take(&mut announced));
                        let _ = socket.write_all(&EMPTY_SETTINGS).await;
                    }
                }
                PING if flags & ACK == 0 => {
                    if let Some(pinged) = pinged.take() {
                        let _ = pinged.send(());
                    }
                }
                _ => {}
            }
        }
    });
    endpoint
}

fn config(endpoint: &str, http2: Http2Config) -> GrpcChannelConfig {
    let mut transport = TransportConfig::new(Uri::try_from(endpoint).expect("an endpoint"));
    transport.connect_timeout = Duration::from_secs(5);
    transport.http2 = http2;
    GrpcChannelConfig::new(transport)
}

/// The windows the client announces when its configuration is `http2`.
async fn announced(http2: Http2Config) -> Announced {
    let (heard, hearing) = oneshot::channel();
    let (pinged, _) = oneshot::channel();
    let endpoint = listening(heard, pinged).await;
    let channel = channel_with(config(&endpoint, http2)).expect("a channel");

    let call = tokio::spawn(async move {
        unary(
            &channel,
            CallStartOptions::new(ECHO),
            Bytes::from_static(b"x"),
        )
        .await
    });
    let announced = tokio::time::timeout(Duration::from_secs(5), hearing)
        .await
        .expect("the client reached its first request")
        .expect("the server heard it");
    call.abort();
    announced
}

#[tokio::test]
async fn the_windows_a_channel_is_given_are_the_ones_it_announces() {
    let mut http2 = Http2Config::default();
    http2.stream_window = 1024 * 1024;
    http2.connection_window = 3 * 1024 * 1024;

    let announced = announced(http2).await;
    assert_eq!(announced.stream_window, Some(1024 * 1024), "{announced:?}");
    assert_eq!(
        announced.connection_window,
        Some(3 * 1024 * 1024),
        "{announced:?}"
    );
}

#[tokio::test]
async fn the_default_windows_are_two_and_five_mebibytes() {
    let announced = announced(Http2Config::default()).await;
    assert_eq!(
        announced.stream_window,
        Some(2 * 1024 * 1024),
        "{announced:?}"
    );
    assert_eq!(
        announced.connection_window,
        Some(5 * 1024 * 1024),
        "{announced:?}"
    );
}

#[tokio::test]
async fn a_peer_that_answers_no_ping_ends_the_session_and_the_call_on_it() {
    let (heard, _) = oneshot::channel();
    let (pinged, ping) = oneshot::channel();
    let endpoint = listening(heard, pinged).await;

    let mut http2 = Http2Config::default();
    http2.keep_alive_interval = Some(Duration::from_millis(100));
    http2.keep_alive_timeout = Duration::from_millis(200);
    let channel = channel_with(config(&endpoint, http2)).expect("a channel");

    let (_, _, status) = tokio::time::timeout(
        Duration::from_secs(10),
        unary(
            &channel,
            CallStartOptions::new(ECHO),
            Bytes::from_static(b"x"),
        ),
    )
    .await
    .expect("the unanswered PING ended the call");
    assert_eq!(status.code, GrpcStatusCode::Unavailable, "{status}");
    assert!(ping.await.is_ok(), "the server was sent a PING");
}

#[tokio::test]
async fn a_setting_no_session_could_use_is_refused() {
    let endpoint = "http://127.0.0.1:1";
    let changes: [fn(&mut Http2Config); 7] = [
        |http2| http2.stream_window = 0,
        |http2| http2.connection_window = 1 << 31,
        |http2| http2.connection_window = 65_534,
        |http2| http2.keep_alive_interval = Some(Duration::ZERO),
        |http2| http2.keep_alive_timeout = Duration::ZERO,
        |http2| http2.idle_timeout = Some(Duration::ZERO),
        |http2| http2.send_buffer = 0,
    ];
    let refusals = changes.map(|change| {
        let mut http2 = Http2Config::default();
        change(&mut http2);
        http2
    });
    for http2 in refusals {
        assert!(
            channel_with(config(endpoint, http2)).is_err(),
            "{http2:?} is admitted"
        );
    }
}

/// The socket options themselves are not readable from here, so what is pinned is that one no
/// socket could take is refused before a dial.
#[tokio::test]
async fn a_tcp_keepalive_no_socket_could_use_is_refused() {
    let changes: [fn(&mut TransportConfig); 7] = [
        |transport| transport.tcp.keepalive = Some(Duration::from_secs(32768)),
        |transport| {
            transport.tcp.keepalive = Some(Duration::from_secs(30));
            transport.tcp.keepalive_retries = Some(128);
        },
        |transport| transport.tcp.keepalive = Some(Duration::from_millis(500)),
        |transport| {
            transport.tcp.keepalive = Some(Duration::from_secs(30));
            transport.tcp.keepalive_interval = Some(Duration::from_millis(500));
        },
        |transport| {
            transport.tcp.keepalive = Some(Duration::from_secs(30));
            transport.tcp.keepalive_retries = Some(0);
        },
        |transport| transport.tcp.keepalive_interval = Some(Duration::from_secs(5)),
        |transport| transport.tcp.keepalive_retries = Some(3),
    ];
    for change in changes {
        let mut transport = TransportConfig::new(Uri::from_static("http://127.0.0.1:1"));
        change(&mut transport);
        let refused = channel_with(GrpcChannelConfig::new(transport.clone()));
        assert!(refused.is_err(), "{transport:?} is admitted");
    }
}

#[tokio::test]
async fn settings_at_their_bounds_are_admitted() {
    let mut transport = TransportConfig::new(Uri::from_static("http://127.0.0.1:1"));
    transport.tcp.keepalive = Some(Duration::from_secs(30));
    transport.tcp.keepalive_interval = Some(Duration::from_secs(1));
    transport.tcp.keepalive_retries = Some(3);
    transport.http2.stream_window = 1;
    transport.http2.connection_window = 65_535;
    transport.http2.keep_alive_interval = Some(Duration::from_millis(1));
    transport.http2.send_buffer = u32::MAX as usize;
    channel_with(GrpcChannelConfig::new(transport.clone())).expect("every setting is in bounds");

    transport.http2.send_buffer = 1;
    channel_with(GrpcChannelConfig::new(transport)).expect("every setting is in bounds");
}

/// The largest send buffer hyper takes is one a session is opened with.
#[tokio::test]
async fn the_largest_send_buffer_opens_a_session() {
    let mut http2 = Http2Config::default();
    http2.send_buffer = u32::MAX as usize;
    announced(http2).await;
}

/// Past `u32::MAX`, where hyper would panic, which only a 64-bit `usize` can name.
#[cfg(target_pointer_width = "64")]
#[tokio::test]
async fn a_send_buffer_past_what_hyper_takes_is_refused() {
    let mut http2 = Http2Config::default();
    http2.send_buffer = u32::MAX as usize + 1;
    assert!(channel_with(config("http://127.0.0.1:1", http2)).is_err());
}
