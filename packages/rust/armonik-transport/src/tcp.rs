//! Establishing the TCP connection under the tunnel and the TLS layer.
//!
//! Two connectors live here. [`TcpConnector::Standard`] is hyper's own, used whenever port reuse is
//! off, so the default path is byte for byte the one this crate has always taken.
//! [`TcpConnector::ReusePorts`] is used only when `reuse_ports` is set: it owns socket creation,
//! which is the only way to set `SO_REUSE_UNICASTPORT` before connecting.
//!
//! Splitting it this way is deliberate. The reuse path is newer code, and confining it to the
//! opt-in case keeps every existing client on the well-travelled one.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use hyper::http::uri::Scheme;
use hyper::Uri;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioIo;
use tokio::net::{TcpSocket, TcpStream};
use tower_service::Service;

use super::ClientConfig;

/// Boxed error, matching what the connectors below and hyper itself produce.
type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Await `body`, bounded by an absolute `deadline` when one is set. [`None`] means it expired.
async fn with_optional_deadline<T>(
    deadline: Option<tokio::time::Instant>,
    body: impl std::future::Future<Output = T>,
) -> Option<T> {
    match deadline {
        Some(deadline) => tokio::time::timeout_at(deadline, body).await.ok(),
        None => Some(body.await),
    }
}

/// How the underlying TCP connection is opened.
#[derive(Debug, Clone)]
pub enum TcpConnector {
    /// hyper's connector. The default, and unchanged.
    Standard(HttpConnector),
    /// Our own connector, which can defer ephemeral port allocation.
    ReusePorts(ReusePortsConnector),
}

impl TcpConnector {
    /// Pick the connector the configuration calls for.
    pub(crate) fn new(config: &ClientConfig) -> Self {
        let mut http = HttpConnector::new();
        http.enforce_http(false); // required for hyper-rustls to switch schemes
        http.set_nodelay(!config.tcp_nagle_algorithm);
        http.set_keepalive(config.tcp_keepalive);
        http.set_keepalive_interval(config.tcp_keepalive_interval);
        http.set_keepalive_retries(config.tcp_keepalive_retries);
        if let Some(timeout) = config.connect_timeout {
            http.set_connect_timeout(Some(timeout));
        }

        if !config.reuse_ports {
            return Self::Standard(http);
        }

        Self::ReusePorts(ReusePortsConnector {
            nodelay: !config.tcp_nagle_algorithm,
            keepalive: config.tcp_keepalive,
            keepalive_interval: config.tcp_keepalive_interval,
            keepalive_retries: config.tcp_keepalive_retries,
            connect_timeout: config.connect_timeout,
        })
    }
}

impl Service<Uri> for TcpConnector {
    type Response = TokioIo<TcpStream>;
    type Error = BoxError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        match self {
            Self::Standard(inner) => inner.poll_ready(cx).map_err(Into::into),
            Self::ReusePorts(inner) => inner.poll_ready(cx),
        }
    }

    fn call(&mut self, target: Uri) -> Self::Future {
        match self {
            Self::Standard(inner) => {
                let future = inner.call(target);
                Box::pin(async move { future.await.map_err(Into::into) })
            }
            Self::ReusePorts(inner) => inner.call(target),
        }
    }
}

/// A TCP connector that asks the OS to defer ephemeral port allocation.
///
/// Opening many short-lived connections on Windows can exhaust the ephemeral port range;
/// `SO_REUSE_UNICASTPORT` lets several outbound connections share a local port when their remote
/// endpoints differ. The option only exists from Windows 10 and Server 2016 onwards, and this
/// transport specifically targets older machines, so a rejection is logged and ignored rather than
/// failing the connection.
#[derive(Debug, Clone)]
pub(crate) struct ReusePortsConnector {
    nodelay: bool,
    keepalive: Option<Duration>,
    keepalive_interval: Option<Duration>,
    keepalive_retries: Option<u32>,
    connect_timeout: Option<Duration>,
}

impl Service<Uri> for ReusePortsConnector {
    type Response = TokioIo<TcpStream>;
    type Error = BoxError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, target: Uri) -> Self::Future {
        let connector = self.clone();
        Box::pin(async move { connector.connect(target).await })
    }
}

impl ReusePortsConnector {
    async fn connect(&self, target: Uri) -> Result<TokioIo<TcpStream>, BoxError> {
        let host = target
            .host()
            .ok_or_else(|| BoxError::from(format!("`{target}` has no host to connect to")))?
            // An IPv6 literal is bracketed in a URI but must not be when resolving.
            .trim_matches(['[', ']']);
        let port = target.port_u16().unwrap_or_else(|| {
            if target.scheme() == Some(&Scheme::HTTPS) {
                443
            } else {
                80
            }
        });

        // Anchored before resolution, so name lookup and every connection attempt draw from the one
        // budget the caller asked for. Bounding each attempt separately instead would let a host
        // with several addresses take a multiple of `connect_timeout` to fail.
        let deadline = self
            .connect_timeout
            .map(|timeout| tokio::time::Instant::now() + timeout);

        let addresses = with_optional_deadline(deadline, tokio::net::lookup_host((host, port)))
            .await
            .ok_or_else(|| BoxError::from(format!("resolving `{host}` timed out")))?
            .map_err(|source| BoxError::from(format!("could not resolve `{host}`: {source}")))?
            .collect::<Vec<_>>();

        if addresses.is_empty() {
            return Err(BoxError::from(format!("`{host}` resolved to no address")));
        }

        let mut last_error = None;
        for address in addresses {
            let attempt = with_optional_deadline(deadline, self.connect_to(address)).await;
            match attempt {
                Some(Ok(stream)) => return Ok(TokioIo::new(stream)),
                Some(Err(error)) => {
                    tracing::debug!(%address, %error, "Connection attempt failed");
                    last_error = Some(error);
                }
                None => {
                    return Err(BoxError::from(format!(
                        "connecting to `{host}` timed out after trying {address}"
                    )))
                }
            }
        }

        Err(last_error.unwrap_or_else(|| BoxError::from(format!("could not connect to `{host}`"))))
    }

    async fn connect_to(&self, address: std::net::SocketAddr) -> Result<TcpStream, BoxError> {
        let socket = if address.is_ipv4() {
            TcpSocket::new_v4()
        } else {
            TcpSocket::new_v6()
        }?;

        // Must happen before `connect`: the option governs how the local port is picked.
        if let Err(error) = enable_port_reuse(&socket) {
            tracing::debug!(
                %error,
                "The OS refused SO_REUSE_UNICASTPORT; connecting without deferred port allocation"
            );
        }

        socket.set_nodelay(self.nodelay)?;
        socket.set_keepalive(self.keepalive.is_some())?;

        // No timeout here: `connect` above bounds the whole operation against one deadline, and a
        // second per-attempt timeout would be the very thing that let the total overrun it.
        let stream = socket.connect(address).await?;

        self.apply_keepalive(&stream)?;

        Ok(stream)
    }

    /// Apply the fine-grained keepalive settings, so turning port reuse on does not quietly drop
    /// them compared with the standard connector.
    fn apply_keepalive(&self, stream: &TcpStream) -> Result<(), BoxError> {
        let Some(time) = self.keepalive else {
            return Ok(());
        };

        let mut keepalive = socket2::TcpKeepalive::new().with_time(time);
        if let Some(interval) = self.keepalive_interval {
            keepalive = keepalive.with_interval(interval);
        }
        if let Some(retries) = self.keepalive_retries {
            keepalive = with_retries(keepalive, retries);
        }

        socket2::SockRef::from(stream).set_tcp_keepalive(&keepalive)?;
        Ok(())
    }
}

/// Set the keepalive retry count where the platform allows it.
///
/// Mirrors what hyper's own connector does, so turning port reuse on does not change which settings
/// take effect: Windows and Apple platforms do not expose this knob, and both connectors ignore it
/// there rather than failing.
#[cfg(not(any(target_os = "windows", target_vendor = "apple")))]
fn with_retries(keepalive: socket2::TcpKeepalive, retries: u32) -> socket2::TcpKeepalive {
    keepalive.with_retries(retries)
}

#[cfg(any(target_os = "windows", target_vendor = "apple"))]
fn with_retries(keepalive: socket2::TcpKeepalive, _retries: u32) -> socket2::TcpKeepalive {
    keepalive
}

/// Ask the OS to defer ephemeral port allocation for this socket.
///
/// Only Windows has such an option; elsewhere this is a no-op and `reuse_ports` has no effect, which
/// is what the option documents.
#[cfg(windows)]
fn enable_port_reuse(socket: &TcpSocket) -> std::io::Result<()> {
    use std::os::windows::io::{AsRawSocket, RawSocket};

    /// `SOL_SOCKET`, from the Windows SDK's `winsock.h`.
    const SOL_SOCKET: i32 = 0xffff;
    /// `SO_REUSE_UNICASTPORT`, from the Windows SDK's `ws2def.h`.
    const SO_REUSE_UNICASTPORT: i32 = 0x3007;

    // `RawSocket` is `u32` on 32-bit and `u64` on 64-bit, matching Winsock's `UINT_PTR`, so this
    // declaration is correct for both architectures this crate is built for.
    #[link(name = "ws2_32")]
    extern "system" {
        fn setsockopt(
            socket: RawSocket,
            level: i32,
            option: i32,
            value: *const u8,
            length: i32,
        ) -> i32;
    }

    let enabled: u32 = 1;

    // SAFETY: `socket` keeps the handle valid for the duration of the call, and `value`/`length`
    // describe the 4-byte DWORD that this option expects.
    let result = unsafe {
        setsockopt(
            socket.as_raw_socket(),
            SOL_SOCKET,
            SO_REUSE_UNICASTPORT,
            std::ptr::addr_of!(enabled).cast::<u8>(),
            std::mem::size_of_val(&enabled) as i32,
        )
    };

    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(windows))]
fn enable_port_reuse(_socket: &TcpSocket) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(reuse_ports: bool) -> ClientConfig {
        let mut config = ClientConfig::default();
        config.reuse_ports = reuse_ports;
        config
    }

    #[test]
    fn the_standard_connector_is_used_unless_port_reuse_is_asked_for() {
        assert!(matches!(
            TcpConnector::new(&config(false)),
            TcpConnector::Standard(_)
        ));
        assert!(matches!(
            TcpConnector::new(&config(true)),
            TcpConnector::ReusePorts(_)
        ));
    }

    #[test]
    fn the_reuse_connector_carries_the_tcp_settings_over() {
        let mut config = config(true);
        config.tcp_nagle_algorithm = true;
        config.tcp_keepalive = Some(Duration::from_secs(30));
        config.tcp_keepalive_interval = Some(Duration::from_secs(5));
        config.tcp_keepalive_retries = Some(3);
        config.connect_timeout = Some(Duration::from_secs(7));

        let TcpConnector::ReusePorts(connector) = TcpConnector::new(&config) else {
            panic!("expected the port-reuse connector");
        };

        // Turning port reuse on must not silently drop settings the standard connector honours.
        assert!(!connector.nodelay, "Nagle enabled means nodelay off");
        assert_eq!(connector.keepalive, Some(Duration::from_secs(30)));
        assert_eq!(connector.keepalive_interval, Some(Duration::from_secs(5)));
        assert_eq!(connector.keepalive_retries, Some(3));
        assert_eq!(connector.connect_timeout, Some(Duration::from_secs(7)));
    }

    /// Winsock must accept `SO_REUSE_UNICASTPORT` on a fresh socket.
    ///
    /// The option cannot be read back — `getsockopt` answers 0 whatever was set — so success is all
    /// there is to observe. On its own that would be weak evidence, since an ignored option number
    /// could also report success, which is what the companion test below rules out.
    #[cfg(windows)]
    #[test]
    fn winsock_accepts_the_port_reuse_option() {
        let socket = TcpSocket::new_v4().expect("socket");

        enable_port_reuse(&socket)
            .expect("Windows 10 and Server 2016 onwards should accept SO_REUSE_UNICASTPORT");
    }

    /// The control for the test above: an option number Winsock does not know must be rejected.
    ///
    /// Without this, a wrong constant or a mismatched calling convention would look exactly like
    /// success, and port reuse would silently never happen.
    #[cfg(windows)]
    #[test]
    fn winsock_rejects_an_unknown_option() {
        use std::os::windows::io::{AsRawSocket, RawSocket};

        const SOL_SOCKET: i32 = 0xffff;
        /// Deliberately not a real option.
        const NONSENSE_OPTION: i32 = 0x3fff;

        #[link(name = "ws2_32")]
        extern "system" {
            fn setsockopt(
                socket: RawSocket,
                level: i32,
                option: i32,
                value: *const u8,
                length: i32,
            ) -> i32;
        }

        let socket = TcpSocket::new_v4().expect("socket");
        let enabled: u32 = 1;

        // SAFETY: same contract as `enable_port_reuse`, with an option number chosen to fail.
        let result = unsafe {
            setsockopt(
                socket.as_raw_socket(),
                SOL_SOCKET,
                NONSENSE_OPTION,
                std::ptr::addr_of!(enabled).cast::<u8>(),
                std::mem::size_of_val(&enabled) as i32,
            )
        };

        assert_ne!(
            result, 0,
            "Winsock accepted a nonsense option, so accepting SO_REUSE_UNICASTPORT proves nothing"
        );
    }

    /// The option governs how the local port is chosen, so it only applies before the socket is
    /// bound. Setting it late fails, which is why the connector sets it on a fresh socket.
    #[cfg(windows)]
    #[tokio::test]
    async fn the_port_reuse_option_must_be_set_before_binding() {
        let socket = TcpSocket::new_v4().expect("socket");
        socket
            .bind("127.0.0.1:0".parse().expect("address"))
            .expect("bind");

        assert!(
            enable_port_reuse(&socket).is_err(),
            "a bound socket should reject the option"
        );
    }

    #[tokio::test]
    async fn a_host_that_resolves_to_nothing_is_reported() {
        let connector = ReusePortsConnector {
            nodelay: true,
            keepalive: None,
            keepalive_interval: None,
            keepalive_retries: None,
            connect_timeout: Some(Duration::from_secs(1)),
        };

        let target = Uri::try_from("http://invalid.invalid:1/").expect("uri");
        let error = connector
            .connect(target)
            .await
            .expect_err("an unresolvable host must fail");

        assert!(
            error.to_string().contains("invalid.invalid"),
            "the error should name the host: {error}"
        );
    }
}
