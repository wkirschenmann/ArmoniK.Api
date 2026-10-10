//! The gRPC client engine ArmoniK's clients reach a server with.
//!
//! [`grpc`] over [`http2`] is the engine: tonic's client, which carries the gRPC framing, over an
//! HTTP/2 session of this crate's own, in cleartext or over TLS. It is configured with
//! [`options::ChannelOptions`], where the schema states each setting's bounds and the generated
//! .NET class its spelling, settled into the engine's configuration by
//! [`settings::ChannelSettings`]. [`configuration::Configuration`] loads the options from files, the
//! environment and documents.

mod coalesce;
pub mod configuration;
mod connector;
pub mod grpc;
#[cfg(feature = "test-hooks")]
pub mod hooks;
pub mod http2;
pub mod metrics;
pub mod options;
mod proxy;
pub mod settings;
mod tls;
mod utils;
#[cfg(windows)]
mod windows_proxy;
#[cfg(windows)]
mod windows_store;

pub use connector::{https_connector, ConnectionError};
pub use utils::safe_endpoint;

pub mod reexports {
    pub use bytes;
    pub use http;
    pub use hyper;
    pub use hyper_rustls;
    pub use hyper_util;
    pub use rustls;
    pub use serde;
    /// Needed to read an error's causes: the outer message of a `ConnectionError` names the step
    /// that failed, and `snafu::Report` is what prints the chain under it. Already a public
    /// dependency through the error types themselves.
    pub use snafu;
    pub use tonic;
}
