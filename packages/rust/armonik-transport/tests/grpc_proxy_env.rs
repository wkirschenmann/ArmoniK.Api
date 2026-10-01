//! A call through the proxy the environment names: which variable is read, what `NO_PROXY` and a
//! loopback endpoint go around, and how its credentials meet the dedicated ones.
//!
//! The variables are the process's, so every test here is serialised and restores them.

mod common;

use std::ffi::OsString;
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
use serial_test::serial;

/// `alice:s3cret`, as `Basic` writes it.
const ALICE: &str = "YWxpY2U6czNjcmV0";

const PROXY_VARIABLES: [&str; 9] = [
    "ALL_PROXY",
    "all_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "NO_PROXY",
    "no_proxy",
    "REQUEST_METHOD",
];

/// The proxy variables as a test sets them, every other one unset, and all of them put back as
/// they were when it is dropped.
struct Environment(Vec<(&'static str, Option<OsString>)>);

impl Environment {
    fn with(set: &[(&str, &str)]) -> Self {
        let saved = PROXY_VARIABLES
            .iter()
            .map(|&name| (name, std::env::var_os(name)))
            .collect();
        for name in PROXY_VARIABLES {
            std::env::remove_var(name);
        }
        for (name, value) in set {
            std::env::set_var(name, value);
        }
        Self(saved)
    }
}

impl Drop for Environment {
    fn drop(&mut self) {
        for (name, value) in &self.0 {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}

fn from_environment(endpoint: &str, username: &str, password: &str) -> TransportConfig {
    let mut transport = TransportConfig::new(Uri::try_from(endpoint).expect("an endpoint"));
    transport.connect_timeout = Duration::from_secs(5);
    let mut proxy = ProxyConfig::default();
    proxy.source = ProxySource::System;
    proxy.username = username.to_owned();
    proxy.password = password.to_owned().into();
    transport.proxy = proxy;
    transport
}

async fn echo(transport: TransportConfig) -> GrpcStatus {
    let channel = channel_with(GrpcChannelConfig::new(transport)).expect("a channel");
    let (_, _, status) = unary(
        &channel,
        CallStartOptions::new(ECHO),
        Bytes::from_static(b"hello"),
    )
    .await;
    status
}

/// `http://127.0.0.1:<port>` under a `.test` name, which the test proxy reaches and a direct dial
/// does not resolve.
fn under_a_name(endpoint: &str) -> String {
    endpoint.replace("127.0.0.1", "server.test")
}

#[tokio::test]
#[serial]
async fn the_proxy_the_environment_names_is_tunnelled_through() {
    let server = TestServer::start().await;
    let proxy = TestProxy::reaching_test_names(Demands::Nothing).await;
    let endpoint = under_a_name(&server.endpoint);

    for variable in ["HTTP_PROXY", "http_proxy", "ALL_PROXY"] {
        let _environment = Environment::with(&[(variable, &proxy.uri)]);
        let status = echo(from_environment(&endpoint, "", "")).await;
        assert_eq!(status.code, GrpcStatusCode::Ok, "{variable}: {status}");
    }
    assert_eq!(proxy.tunnels(), 3);
    assert!(proxy
        .asked()
        .iter()
        .all(|target| target.starts_with("server.test:")));
}

#[tokio::test]
#[serial]
async fn an_https_endpoint_goes_through_the_https_proxy() {
    let pki = Pki::new();
    let server = TlsServer::start(pki.server(&["server.test"]), None).await;
    let proxy = TestProxy::reaching_test_names(Demands::Nothing).await;
    let dead = closed_port().await;
    let _environment = Environment::with(&[("HTTPS_PROXY", &proxy.uri), ("HTTP_PROXY", &dead)]);

    let mut transport = from_environment(&under_a_name(&server.endpoint), "", "");
    let mut tls = TlsConfig::default();
    tls.roots = vec![pki.root()];
    transport.tls = tls;
    let status = echo(transport).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(proxy.tunnels(), 1);
}

#[tokio::test]
#[serial]
async fn a_host_no_proxy_names_is_dialled_directly() {
    let proxy = TestProxy::start(Demands::Nothing).await;
    let _environment =
        Environment::with(&[("HTTP_PROXY", &proxy.uri), ("NO_PROXY", "server.test")]);

    // Directly, the name resolves to nothing: the call fails, and never reaches the proxy.
    let status = echo(from_environment("http://server.test:1", "", "")).await;
    assert_eq!(status.code, GrpcStatusCode::Unavailable, "{status}");
    assert!(!status.message.contains("proxy"), "{status}");
    assert!(proxy.asked().is_empty(), "{:?}", proxy.asked());
}

#[tokio::test]
#[serial]
async fn a_loopback_endpoint_is_never_proxied() {
    let server = TestServer::start().await;
    let proxy = TestProxy::start(Demands::Nothing).await;
    let _environment = Environment::with(&[("HTTP_PROXY", &proxy.uri)]);

    let port = server.endpoint.rsplit(':').next().expect("a port");
    for endpoint in [
        server.endpoint.clone(),
        format!("http://localhost:{port}"),
        format!("http://LocalHost.:{port}"),
    ] {
        let status = echo(from_environment(&endpoint, "", "")).await;
        assert_eq!(status.code, GrpcStatusCode::Ok, "{endpoint}: {status}");
    }
    assert!(proxy.asked().is_empty(), "{:?}", proxy.asked());
}

#[tokio::test]
#[serial]
async fn no_variable_is_a_direct_dial() {
    let server = TestServer::start().await;
    let _environment = Environment::with(&[]);

    let status = echo(from_environment(&server.endpoint, "alice", "s3cret")).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
}

#[tokio::test]
#[serial]
async fn a_proxy_the_environment_names_by_another_scheme_is_refused_when_the_channel_is_made() {
    for url in [
        "https://alice:s3cret@proxy.test:3128",
        "socks5h://alice:s3cret@proxy.test:1080",
    ] {
        let _environment = Environment::with(&[("HTTP_PROXY", url)]);
        let refused = channel_with(GrpcChannelConfig::new(from_environment(
            "http://server.test:1",
            "",
            "",
        )))
        .expect_err(url);
        let GrpcChannelConfigError::Transport { source } = &refused else {
            panic!("{refused:?}");
        };
        assert_eq!(source.kind(), TransportErrorKind::Configuration);
        let message = refused.to_string();
        assert!(message.contains("not an `http://` URL"), "{message}");
        assert!(!message.contains("s3cret"), "{message}");
    }
}

#[tokio::test]
#[serial]
async fn the_dedicated_credentials_take_the_place_of_the_urls_half_by_half() {
    let server = TestServer::start().await;
    let proxy = TestProxy::reaching_test_names(Demands::Credentials(ALICE)).await;
    let endpoint = under_a_name(&server.endpoint);
    let at = |userinfo: &str| proxy.uri.replace("http://", &format!("http://{userinfo}@"));

    for (url, username, password) in [
        (at("alice:s3cret"), "", ""),
        (at("alice:hunter2"), "", "s3cret"),
        (at("bob:s3cret"), "alice", ""),
        (proxy.uri.clone(), "alice", "s3cret"),
    ] {
        let _environment = Environment::with(&[("HTTP_PROXY", &url)]);
        let status = echo(from_environment(&endpoint, username, password)).await;
        assert_eq!(
            status.code,
            GrpcStatusCode::Ok,
            "{url} {username}: {status}"
        );
    }

    let _environment = Environment::with(&[("HTTP_PROXY", &at("alice:hunter2"))]);
    let status = echo(from_environment(&endpoint, "", "")).await;
    assert!(status.message.contains("asks for credentials"), "{status}");
    assert!(!status.message.contains("hunter2"), "{status}");
}
