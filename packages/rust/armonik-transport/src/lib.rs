//! The gRPC client engine ArmoniK's clients reach a server with.
//!
//! [`grpc`] over [`http2`] is the engine: tonic's client, which carries the gRPC framing, over an
//! HTTP/2 session of this crate's own, in cleartext or over TLS. The C ABI configures it with an
//! [`options::ChannelOptions`] document, where the schema states each setting's bounds and the
//! generated .NET class its spelling. The Rust client configures it with a [`ClientConfig`], read
//! from the `GrpcClient__*` environment, through [`ClientConfig::channel_config`], which refuses a
//! setting the engine has not got rather than drop it.
//!
//! [`connect`] builds a `tonic` channel over `hyper-rustls` from the same [`ClientConfig`].

mod coalesce;
mod config;
mod connect;
pub mod grpc;
#[cfg(feature = "test-hooks")]
pub mod hooks;
pub mod http2;
pub mod options;
mod proxy;
mod tls;
mod utils;
#[cfg(windows)]
mod windows_proxy;
#[cfg(windows)]
mod windows_store;

pub use config::{ClientConfig, ClientConfigArgs, ConfigError};
pub use connect::{connect, https_connector, ConnectionError};
#[doc(hidden)]
pub use connect::{ConfigSnafu, IoSnafu, TlsSnafu, TransportSnafu};
pub use utils::{safe_endpoint, ReadEnvError};

pub mod reexports {
    pub use bytes;
    pub use http;
    pub use hyper;
    pub use hyper_rustls;
    pub use hyper_util;
    pub use rustls;
    #[cfg(feature = "serde")]
    pub use serde;
    /// Needed to read an error's causes: the outer message of a `ConfigError` or a
    /// `ConnectionError` names the step that failed, and `snafu::Report` is what prints the
    /// chain under it. Already a public dependency through the error types themselves.
    pub use snafu;
    pub use tonic;
}
