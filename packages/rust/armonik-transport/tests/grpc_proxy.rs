//! A call through an explicit HTTP proxy, by a `CONNECT` tunnel: in the clear and over TLS, with
//! and without credentials, and every way the proxy can fail without the call going around it.

mod common;

use std::time::Duration;

use armonik_transport::grpc::{
    CallStartOptions, GrpcChannelConfig, GrpcChannelConfigError, GrpcStatus, GrpcStatusCode,
};
use armonik_transport::http2::{
    ProxyConfig, ProxySource, TlsConfig, TransportConfig, TransportErrorKind,
};
use bytes::Bytes;
use common::echo::{channel_with, closed_port, unary, TestServer, ECHO};
use common::proxy::{Demands, TestProxy};
use common::tls::{Pki, TlsServer};
use http::Uri;

/// `alice:s3cret`, as `Basic` writes it.
const ALICE: &str = "YWxpY2U6czNjcmV0";

fn proxied(endpoint: &str, proxy: &str, credentials: Option<(&str, &str)>) -> TransportConfig {
    let mut transport = TransportConfig::new(Uri::try_from(endpoint).expect("an endpoint"));
    transport.connect_timeout = Duration::from_secs(5);
    let mut config = ProxyConfig::default();
    config.source = ProxySource::Explicit(Uri::try_from(proxy).expect("a proxy"));
    if let Some((username, password)) = credentials {
        config.username = username.to_owned();
        config.password = password.to_owned().into();
    }
    transport.proxy = config;
    transport
}

async fn echo(transport: TransportConfig) -> GrpcStatus {
    let channel = channel_with(GrpcChannelConfig::new(transport)).expect("a channel");
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

#[tokio::test]
async fn a_call_in_the_clear_goes_through_the_tunnel() {
    let server = TestServer::start().await;
    let proxy = TestProxy::start(Demands::Nothing).await;

    let status = echo(proxied(&server.endpoint, &proxy.uri, None)).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(proxy.tunnels(), 1);
}

/// The port a direct dial would take: `Tunnel` itself would ask for 443 whatever the scheme.
#[tokio::test]
async fn an_endpoint_naming_no_port_is_tunnelled_to_its_schemes_port() {
    let proxy = TestProxy::start(Demands::Nothing).await;

    let _ = echo(proxied("http://127.0.0.1", &proxy.uri, None)).await;
    assert_eq!(proxy.asked(), vec!["127.0.0.1:80".to_owned()]);
}

#[tokio::test]
async fn a_call_over_tls_goes_through_the_tunnel_with_tls_end_to_end() {
    let pki = Pki::new();
    let server = TlsServer::start(pki.server(&["127.0.0.1"]), None).await;
    let proxy = TestProxy::start(Demands::Nothing).await;

    let mut transport = proxied(&server.endpoint, &proxy.uri, None);
    let mut tls = TlsConfig::default();
    tls.roots = vec![pki.root()];
    transport.tls = tls;
    let status = echo(transport).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(proxy.tunnels(), 1);
}

#[tokio::test]
async fn credentials_are_presented_to_a_proxy_that_asks_for_them() {
    let server = TestServer::start().await;
    let proxy = TestProxy::start(Demands::Credentials(ALICE)).await;

    let status = echo(proxied(
        &server.endpoint,
        &proxy.uri,
        Some(("alice", "s3cret")),
    ))
    .await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
}

#[tokio::test]
async fn a_proxy_demanding_credentials_it_was_not_given_says_so() {
    let server = TestServer::start().await;
    let proxy = TestProxy::start(Demands::Credentials(ALICE)).await;

    for credentials in [None, Some(("alice", "hunter2"))] {
        let status = echo(proxied(&server.endpoint, &proxy.uri, credentials)).await;
        assert_eq!(status.code, GrpcStatusCode::Unavailable, "{status}");
        assert!(status.message.contains("asks for credentials"), "{status}");
        assert!(!status.message.contains("hunter2"), "{status}");
    }
    assert_eq!(proxy.tunnels(), 0);
}

#[tokio::test]
async fn a_proxy_out_of_reach_fails_the_call_rather_than_being_gone_around() {
    let server = TestServer::start().await;
    let dead = closed_port().await;

    let status = echo(proxied(&server.endpoint, &dead, None)).await;
    assert_eq!(status.code, GrpcStatusCode::Unavailable, "{status}");
    assert!(status.message.contains("through its proxy"), "{status}");
    assert_eq!(server.connections(), 0, "the server was dialled directly");
}

#[tokio::test]
async fn a_proxy_that_never_answers_is_bounded_by_the_connect_timeout() {
    let server = TestServer::start().await;
    let proxy = TestProxy::start(Demands::Silence).await;

    let mut transport = proxied(&server.endpoint, &proxy.uri, None);
    transport.connect_timeout = Duration::from_millis(300);
    let status = tokio::time::timeout(Duration::from_secs(10), echo(transport))
        .await
        .expect("the connect timeout ended the dial");
    assert_eq!(status.code, GrpcStatusCode::Unavailable, "{status}");
}

#[tokio::test]
async fn a_proxy_this_connector_cannot_use_is_refused_when_the_channel_is_made() {
    for (proxy, said) in [
        ("https://proxy.test:443", "`http://`"),
        ("http://alice:s3cret@proxy.test:3128", "user:password@"),
    ] {
        let refused = channel_with(GrpcChannelConfig::new(proxied(
            "http://127.0.0.1:1",
            proxy,
            None,
        )))
        .expect_err(proxy);
        let GrpcChannelConfigError::Transport { source } = &refused else {
            panic!("{refused:?}");
        };
        assert_eq!(source.kind(), TransportErrorKind::Configuration);
        let message = refused.to_string();
        assert!(message.contains(said), "{message}");
        assert!(!message.contains("s3cret"), "{message}");
    }
}
