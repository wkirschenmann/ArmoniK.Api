//! A proxy that only implements `CONNECT`, on an ephemeral loopback port.
//!
//! A few dozen lines here rather than an external binary, so the tests run wherever CI does.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// What the proxy asks of a client before it opens a tunnel.
#[derive(Clone, Copy)]
pub enum Demands {
    Nothing,
    /// A `Proxy-Authorization: Basic` value, base64 of `user:password`.
    Credentials(&'static str),
    /// Never to be answered: the proxy reads the request and says nothing.
    Silence,
}

pub struct TestProxy {
    /// `http://127.0.0.1:<port>`.
    pub uri: String,
    tunnels: Arc<AtomicUsize>,
    asked: Arc<std::sync::Mutex<Vec<String>>>,
}

impl TestProxy {
    pub async fn start(demands: Demands) -> Self {
        Self::spawn(demands, false).await
    }

    /// One that tunnels to a `.test` name as to loopback: a server only the proxy can reach.
    pub async fn reaching_test_names(demands: Demands) -> Self {
        Self::spawn(demands, true).await
    }

    async fn spawn(demands: Demands, test_names: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind the proxy");
        let address: SocketAddr = listener.local_addr().expect("the proxy's address");
        let tunnels = Arc::new(AtomicUsize::new(0));
        let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (opened, heard) = (Arc::clone(&tunnels), Arc::clone(&asked));
        tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                let (opened, heard) = (Arc::clone(&opened), Arc::clone(&heard));
                tokio::spawn(async move {
                    // A refused tunnel is what a test asserts on, not a failure here.
                    let _ = serve(client, demands, test_names, opened, heard).await;
                });
            }
        });
        Self {
            uri: format!("http://{address}"),
            tunnels,
            asked,
        }
    }

    /// The targets the clients asked a tunnel to, as `CONNECT` wrote them.
    pub fn asked(&self) -> Vec<String> {
        self.asked.lock().expect("the targets").clone()
    }

    /// How many tunnels it opened.
    pub fn tunnels(&self) -> usize {
        self.tunnels.load(Ordering::SeqCst)
    }
}

async fn read_head(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte).await?;
        head.push(byte[0]);
        if head.len() > 8 * 1024 {
            return Err(std::io::Error::other("the request head is too large"));
        }
    }
    Ok(String::from_utf8_lossy(&head).into_owned())
}

async fn serve(
    mut client: TcpStream,
    demands: Demands,
    test_names: bool,
    tunnels: Arc<AtomicUsize>,
    asked: Arc<std::sync::Mutex<Vec<String>>>,
) -> std::io::Result<()> {
    let head = read_head(&mut client).await?;
    let target = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or_default()
        .to_owned();
    asked.lock().expect("the targets").push(target.clone());

    match demands {
        Demands::Nothing => {}
        Demands::Silence => {
            let mut rest = Vec::new();
            let _ = client.read_to_end(&mut rest).await;
            return Ok(());
        }
        Demands::Credentials(expected) => {
            let presented = head.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("proxy-authorization")
                    .then(|| value.trim().strip_prefix("Basic ").map(str::to_owned))
                    .flatten()
            });
            if presented.as_deref() != Some(expected) {
                client
                    .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n")
                    .await?;
                return client.flush().await;
            }
        }
    }

    let reached = match target.rsplit_once(':') {
        Some((host, port)) if test_names && host.ends_with(".test") => format!("127.0.0.1:{port}"),
        _ => target,
    };
    let mut upstream = TcpStream::connect(&reached).await?;
    client
        .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
        .await?;
    client.flush().await?;
    tunnels.fetch_add(1, Ordering::SeqCst);
    tokio::io::copy_bidirectional(&mut client, &mut upstream)
        .await
        .map(|_| ())
}
