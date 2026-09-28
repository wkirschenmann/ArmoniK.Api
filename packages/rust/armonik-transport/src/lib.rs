//! Two ways to reach an ArmoniK server, with no configuration in common.
//!
//! [`connect`] builds a `tonic` channel over `hyper-rustls` from a [`ClientConfig`]: TLS, mTLS,
//! the keepalives and every timeout, read from the `GrpcClient__*` environment. It is what the
//! Rust client dials with.
//!
//! [`grpc`] over [`http2`] is the engine the C ABI drives: tonic's client, which carries the gRPC
//! framing, over an HTTP/2 session of this crate's own. Cleartext `http://` only, and configured
//! by an [`options::ChannelOptions`] document rather than by the environment.
//!
//! Nothing converts one configuration into the other, and that is the point: seventeen fields
//! answer to fourteen the engine has no use for, so a conversion would drop them and leave a
//! caller no way to see which of its settings survived. A setting reaches the engine by being
//! named in [`options`], where the schema states its bounds and the generated .NET class its
//! spelling.

mod config;
mod connect;
pub mod grpc;
pub mod http2;
pub mod options;
mod utils;

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
