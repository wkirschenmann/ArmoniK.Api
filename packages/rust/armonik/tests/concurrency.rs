//! Opening many connections in a short window.
//!
//! This mirrors the `MultipleChannels` test of the .NET client suite, which builds up to a hundred
//! channels at once. On Windows that pattern is what exhausts the ephemeral port range, and
//! `GrpcClient__ReusePorts` exists to defer port allocation so it does not.

use std::net::SocketAddr;
use std::sync::Arc;

use armonik::server::{RequestContext, VersionsServiceExt};
use armonik::{versions, ClientConfig};

mod common;

const CORE_VERSION: &str = "concurrent-core-version";

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

async fn spawn_server() -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind server");
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

fn config(endpoint: SocketAddr, reuse_ports: bool) -> ClientConfig {
    let mut config = ClientConfig::default();
    config.endpoint = hyper::Uri::try_from(format!("http://{endpoint}")).expect("endpoint");
    config.reuse_ports = reuse_ports;
    config
}

/// Build `count` independent channels at once and call through every one of them.
///
/// Each channel is its own TCP connection, which is the point: a shared channel would multiplex
/// over one socket and never touch the port range.
async fn open_channels(config: ClientConfig, count: usize) -> Result<(), String> {
    let mut clients = Vec::with_capacity(count);
    for index in 0..count {
        let client = armonik::Client::with_config(config.clone())
            .await
            .map_err(|error| format!("connection {index} failed: {error}"))?;
        clients.push(client.into_versions());
    }

    for (index, client) in clients.iter_mut().enumerate() {
        let response = client
            .list()
            .await
            .map_err(|error| format!("call {index} failed: {error}"))?;
        if response.core != CORE_VERSION {
            return Err(format!("call {index} returned {:?}", response.core));
        }
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn many_channels_without_port_reuse() {
    let server = spawn_server().await;

    open_channels(config(server, false), 100)
        .await
        .expect("100 channels should all connect and answer");
}

#[tokio::test(flavor = "multi_thread")]
async fn many_channels_with_port_reuse() {
    let server = spawn_server().await;

    // Same load, with port reuse on. On Windows this takes the connector that sets
    // `SO_REUSE_UNICASTPORT`; elsewhere the option is accepted and does nothing.
    open_channels(config(server, true), 100)
        .await
        .expect("100 channels should all connect and answer with port reuse on");
}

#[tokio::test(flavor = "multi_thread")]
async fn port_reuse_does_not_change_what_the_call_returns() {
    let server = spawn_server().await;

    for reuse_ports in [false, true] {
        let mut client = armonik::Client::with_config(config(server, reuse_ports))
            .await
            .expect("connect")
            .into_versions();

        assert_eq!(
            client.list().await.expect("call").core,
            CORE_VERSION,
            "reuse_ports={reuse_ports} changed the response"
        );
    }
}
