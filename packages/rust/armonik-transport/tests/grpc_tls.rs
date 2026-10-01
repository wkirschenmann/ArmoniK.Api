//! A call over `https://`, through each branch of the connector: the roots it verifies against,
//! none at all, a name other than the endpoint's, and a client certificate.

mod common;

use std::time::Duration;

use armonik_transport::grpc::{
    CallStartOptions, ChannelError, GrpcChannel, GrpcChannelConfig, GrpcChannelConfigError,
    GrpcStatus, GrpcStatusCode,
};
use armonik_transport::http2::{ClientIdentity, TlsConfig, TransportConfig, TransportErrorKind};
use armonik_transport::reexports::rustls::pki_types::PrivateKeyDer;
use bytes::Bytes;
use common::echo::{channel_with, unary, ECHO};
use common::tls::{Pki, TlsServer};
use http::Uri;

fn channel(endpoint: &str, tls: TlsConfig) -> Result<GrpcChannel, GrpcChannelConfigError> {
    let uri = Uri::try_from(endpoint).expect("the test server's endpoint");
    let mut transport = TransportConfig::new(uri);
    transport.connect_timeout = Duration::from_secs(5);
    transport.tls = tls;
    channel_with(GrpcChannelConfig::new(transport))
}

async fn echo(endpoint: &str, tls: TlsConfig) -> GrpcStatus {
    let channel = channel(endpoint, tls).expect("a channel the configuration admits");
    let (_, messages, status) = unary(
        &channel,
        CallStartOptions::new(ECHO),
        Bytes::from_static(b"hello"),
    )
    .await;
    if status.code == GrpcStatusCode::Ok {
        assert_eq!(messages, vec![Bytes::from_static(b"hello")]);
    }
    status
}

fn trusting(pki: &Pki) -> TlsConfig {
    let mut tls = TlsConfig::default();
    tls.roots = vec![pki.root()];
    tls
}

#[tokio::test]
async fn a_call_over_https_reaches_a_server_whose_root_it_was_given() {
    let pki = Pki::new();
    let server = TlsServer::start(pki.server(&["127.0.0.1"]), None).await;

    let status = echo(&server.endpoint, trusting(&pki)).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
}

#[tokio::test]
async fn the_system_roots_do_not_vouch_for_a_private_authority() {
    let pki = Pki::new();
    let server = TlsServer::start(pki.server(&["127.0.0.1"]), None).await;

    let status = echo(&server.endpoint, TlsConfig::default()).await;
    assert_eq!(status.code, GrpcStatusCode::Unavailable, "{status}");
    assert!(status.message.contains("TLS handshake"), "{status}");
}

#[tokio::test]
async fn accepting_any_server_reaches_one_no_root_vouches_for() {
    let pki = Pki::new();
    let server = TlsServer::start(pki.server(&["127.0.0.1"]), None).await;

    let mut tls = TlsConfig::default();
    tls.accept_any_server = true;
    let status = echo(&server.endpoint, tls).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
}

#[tokio::test]
async fn a_certificate_for_another_name_is_verified_under_the_name_it_was_given() {
    let pki = Pki::new();
    let server = TlsServer::start(pki.server(&["alias.test"]), None).await;

    let refused = echo(&server.endpoint, trusting(&pki)).await;
    assert_eq!(refused.code, GrpcStatusCode::Unavailable, "{refused}");
    assert!(refused.message.contains("TLS handshake"), "{refused}");

    let mut tls = trusting(&pki);
    tls.server_name = Some("alias.test".to_owned());
    let status = echo(&server.endpoint, tls).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
}

#[tokio::test]
async fn a_bracketed_ipv6_server_name_is_verified_as_the_address() {
    let pki = Pki::new();
    let server = TlsServer::start(pki.server(&["::1"]), None).await;

    let mut tls = trusting(&pki);
    tls.server_name = Some("[::1]".to_owned());
    let status = echo(&server.endpoint, tls).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
}

#[tokio::test]
async fn a_server_that_asks_for_a_client_certificate_is_given_one() {
    let pki = Pki::new();
    let server = TlsServer::start(pki.server(&["127.0.0.1"]), Some(&pki)).await;

    // Unavailable and not more: under TLS 1.3 the server refuses a missing certificate after the
    // client's handshake has completed, so the refusal reaches the request and not the dial.
    let refused = echo(&server.endpoint, trusting(&pki)).await;
    assert_eq!(refused.code, GrpcStatusCode::Unavailable, "{refused}");

    let client = pki.client();
    let mut tls = trusting(&pki);
    tls.identity = Some(ClientIdentity {
        chain: client.chain,
        key: client.key,
    });
    let status = echo(&server.endpoint, tls).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
}

#[tokio::test]
async fn a_key_rustls_cannot_read_is_refused_when_the_channel_is_made() {
    let pki = Pki::new();
    let client = pki.client();
    let mut tls = trusting(&pki);
    tls.identity = Some(ClientIdentity {
        chain: client.chain,
        key: PrivateKeyDer::Pkcs8(vec![0u8; 16].into()),
    });

    let refused = channel("https://127.0.0.1:1", tls).expect_err("an unreadable key");
    let GrpcChannelConfigError::Transport { source } = &refused else {
        panic!("{refused:?}");
    };
    assert_eq!(source.kind(), TransportErrorKind::Configuration);
    assert!(
        refused.to_string().contains("client certificate"),
        "{refused}"
    );
}

#[tokio::test]
async fn a_refused_handshake_is_reported_as_one_and_not_as_a_dial() {
    let pki = Pki::new();
    let server = TlsServer::start(pki.server(&["127.0.0.1"]), None).await;
    let channel = channel(&server.endpoint, TlsConfig::default()).expect("a channel");

    let Err(ChannelError::Transport { source }) = channel.connect().await else {
        panic!("a handshake against a private authority completed");
    };
    assert_eq!(source.kind(), TransportErrorKind::TlsHandshake, "{source}");
}
