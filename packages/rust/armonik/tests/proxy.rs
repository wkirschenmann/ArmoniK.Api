//! End-to-end tests for HTTP `CONNECT` tunnelling.
//!
//! These drive a real client through a real proxy to a real gRPC server over loopback sockets, so
//! the handshake, the authentication and the failure paths are all exercised for real. The proxy is
//! a few dozen lines below rather than an external binary, which keeps the tests self-contained and
//! working on every platform CI runs on.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use armonik::server::{RequestContext, VersionsServiceExt};
use armonik::{versions, ClientConfig, ProxyConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

mod common;

/// The version string the stub server reports, used to prove the response really came back.
const CORE_VERSION: &str = "proxied-core-version";

#[derive(Debug, Clone, Default)]
struct Service;

impl armonik::server::VersionsService for Service {
    async fn list(
        self: Arc<Self>,
        _request: versions::list::Request,
        _context: RequestContext,
    ) -> Result<versions::list::Response, tonic::Status> {
        Ok(versions::list::Response {
            core: String::from(CORE_VERSION),
            ..Default::default()
        })
    }
}

/// Serve the stub Versions service on an ephemeral loopback port.
async fn spawn_server() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind server");
    let address = listener.local_addr().expect("server address");

    tokio::spawn(async move {
        let incoming = armonik::reexports::tokio_stream::wrappers::TcpListenerStream::new(listener);
        tonic::transport::Server::builder()
            .add_service(Service.versions_server())
            .serve_with_incoming(incoming)
            .await
            .expect("serve");
    });

    address
}

/// What a test proxy should demand of its clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProxyAuth {
    /// Accept every tunnel request.
    None,
    /// Reject with `407` unless the expected `Proxy-Authorization` header is present.
    Required(&'static str),
}

/// Observations a test can make about what the proxy did.
#[derive(Debug, Default)]
struct ProxyStats {
    /// How many `CONNECT` requests were accepted and tunnelled.
    tunnels: AtomicUsize,
    /// How many `CONNECT` requests were rejected for missing or wrong credentials.
    rejected: AtomicUsize,
}

/// A minimal HTTP proxy that only implements `CONNECT`.
async fn spawn_proxy(auth: ProxyAuth) -> (SocketAddr, Arc<ProxyStats>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind proxy");
    let address = listener.local_addr().expect("proxy address");
    let stats = Arc::new(ProxyStats::default());

    let accepted = Arc::clone(&stats);
    tokio::spawn(async move {
        loop {
            let Ok((client, _)) = listener.accept().await else {
                return;
            };
            let stats = Arc::clone(&accepted);
            tokio::spawn(async move {
                // A failing tunnel is a normal outcome in these tests; the client asserts on it.
                let _ = serve_tunnel(client, auth, stats).await;
            });
        }
    });

    (address, stats)
}

async fn serve_tunnel(
    mut client: TcpStream,
    auth: ProxyAuth,
    stats: Arc<ProxyStats>,
) -> std::io::Result<()> {
    let head = read_head(&mut client).await?;

    let target = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or_default()
        .to_owned();

    if let ProxyAuth::Required(expected) = auth {
        let presented = head.lines().find_map(|line| {
            line.strip_prefix("Proxy-Authorization: Basic ")
                .or_else(|| line.strip_prefix("proxy-authorization: Basic "))
        });

        if presented != Some(expected) {
            stats.rejected.fetch_add(1, Ordering::SeqCst);
            client
                .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n")
                .await?;
            return client.flush().await;
        }
    }

    let mut upstream = TcpStream::connect(&target).await?;
    client
        .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
        .await?;
    client.flush().await?;
    stats.tunnels.fetch_add(1, Ordering::SeqCst);

    tokio::io::copy_bidirectional(&mut client, &mut upstream)
        .await
        .map(|_| ())
}

/// Read up to the blank line that ends an HTTP head.
async fn read_head(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut head = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte).await?;
        head.push(byte[0]);
        if head.ends_with(b"\r\n\r\n") {
            return Ok(String::from_utf8_lossy(&head).into_owned());
        }
        if head.len() > 8 * 1024 {
            return Err(std::io::Error::other("head too large"));
        }
    }
}

fn config(endpoint: SocketAddr, proxy: ProxyConfig) -> ClientConfig {
    let mut config = ClientConfig::default();
    config.endpoint = hyper::Uri::try_from(format!("http://{endpoint}")).expect("endpoint");
    config.proxy = proxy;
    config
}

fn explicit(proxy: SocketAddr) -> ProxyConfig {
    ProxyConfig::explicit(hyper::Uri::try_from(format!("http://{proxy}")).expect("proxy uri"))
}

/// Ask the server for its versions, returning the core version it reported.
async fn call_versions(config: ClientConfig) -> Result<String, Box<dyn std::error::Error>> {
    let mut client = armonik::Client::with_config(config).await?.into_versions();
    Ok(client.list().await?.core)
}

/// Render an error and everything it was caused by.
///
/// The proxy failure is wrapped by the transport error, whose own message is generic, so assertions
/// have to look at the whole chain to see what actually went wrong.
fn error_chain(error: &dyn std::error::Error) -> String {
    let mut rendered = vec![error.to_string()];
    let mut current = error.source();
    while let Some(source) = current {
        rendered.push(source.to_string());
        current = source.source();
    }
    rendered.join(" -> ")
}

#[tokio::test]
async fn request_reaches_the_server_through_the_tunnel() {
    let server = spawn_server().await;
    let (proxy, stats) = spawn_proxy(ProxyAuth::None).await;

    let core = call_versions(config(server, explicit(proxy)))
        .await
        .expect("the call should succeed through the proxy");

    assert_eq!(core, CORE_VERSION);
    assert_eq!(
        stats.tunnels.load(Ordering::SeqCst),
        1,
        "the request must have gone through the proxy, not around it"
    );
}

#[tokio::test]
async fn credentials_are_presented_when_the_proxy_demands_them() {
    let server = spawn_server().await;
    // The base64 of `user:secret`, which is what the client is expected to send.
    let (proxy, stats) = spawn_proxy(ProxyAuth::Required("dXNlcjpzZWNyZXQ=")).await;

    let core = call_versions(config(
        server,
        explicit(proxy).with_credentials("user", "secret"),
    ))
    .await
    .expect("the call should succeed once credentials are supplied");

    assert_eq!(core, CORE_VERSION);
    assert_eq!(stats.tunnels.load(Ordering::SeqCst), 1);
    assert_eq!(stats.rejected.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_missing_credential_is_reported_as_such() {
    let server = spawn_server().await;
    let (proxy, stats) = spawn_proxy(ProxyAuth::Required("dXNlcjpzZWNyZXQ=")).await;

    let error = call_versions(config(server, explicit(proxy)))
        .await
        .expect_err("the proxy should have refused the tunnel");

    // The message has to name the options to set, otherwise a 407 is a dead end for whoever hits
    // it.
    let rendered = error_chain(error.as_ref());
    assert!(
        rendered.contains("requires authentication"),
        "unexpected error: {rendered}"
    );
    assert!(
        rendered.contains("GrpcClient__ProxyUsername"),
        "the error should say which options to set: {rendered}"
    );
    assert_eq!(stats.rejected.load(Ordering::SeqCst), 1);
    assert_eq!(stats.tunnels.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn wrong_credentials_are_rejected() {
    let server = spawn_server().await;
    let (proxy, stats) = spawn_proxy(ProxyAuth::Required("dXNlcjpzZWNyZXQ=")).await;

    let error = call_versions(config(
        server,
        explicit(proxy).with_credentials("user", "wrong"),
    ))
    .await
    .expect_err("the proxy should have refused the tunnel");

    let rendered = error_chain(error.as_ref());
    assert!(
        rendered.contains("requires authentication"),
        "unexpected error: {rendered}"
    );
    assert_eq!(stats.rejected.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_dead_proxy_fails_instead_of_bypassing_it() {
    let server = spawn_server().await;

    // Bind a port and drop it, so nothing is listening there.
    let dead = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind")
        .local_addr()
        .expect("address");
    drop(TcpListener::bind(dead).await);

    let error = call_versions(config(server, explicit(dead)))
        .await
        .expect_err("an unreachable proxy must fail the call");

    // Silently falling back to a direct connection would defeat the point of configuring a proxy.
    let rendered = error_chain(error.as_ref());
    assert!(
        rendered.contains("Could not connect to the proxy"),
        "the error should name the proxy: {rendered}"
    );
}

#[tokio::test]
async fn no_proxy_is_used_when_proxying_is_disabled() {
    let server = spawn_server().await;
    let (proxy, stats) = spawn_proxy(ProxyAuth::None).await;

    let core = call_versions(config(server, ProxyConfig::default()))
        .await
        .expect("a direct call should succeed");

    assert_eq!(core, CORE_VERSION);
    assert_eq!(
        stats.tunnels.load(Ordering::SeqCst),
        0,
        "the proxy must not be involved when it is disabled"
    );
    let _ = proxy;
}
