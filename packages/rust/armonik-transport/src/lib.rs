//! Transport layer for the ArmoniK Rust client.
//!
//! This is "how do I get a channel that speaks to an ArmoniK endpoint" — configuration parsing,
//! TLS/mTLS, the HTTP `CONNECT` proxy tunnel, the optional `SO_REUSE_UNICASTPORT` connector, and
//! the replay policy for retrying a failed request — factored out of the
//! [`armonik`](https://docs.rs/armonik) crate so it can be depended on without pulling in
//! protobuf codegen or any knowledge of ArmoniK's services.
//!
//! `armonik` re-exports everything here at the paths it always had
//! (`armonik::ClientConfig`, `armonik::client::RetryPolicy`, ...), so this split is not a breaking
//! change for it. It exists to serve a second, direct consumer: `armonik-transport-ffi`, the native
//! half of the `ArmoniK.Api.Client.Legacy` .NET Framework binding, needs exactly this layer and
//! nothing else — in particular, no `protoc`/`tonic-prost-build` build step, since nothing here
//! touches a generated proto type.

mod config;
mod connect;
mod proxy;
mod retry;
mod tcp;
mod utils;

pub use config::{
    ClientConfig, ClientConfigArgs, ConfigError, ProxyConfig, ProxySource, RetryPolicy,
};
pub use connect::{connect, https_connector, ConnectionError};
pub use proxy::ProxyError;
pub use retry::{retry_with, MethodKind};
pub use utils::ReadEnvError;

/// Re-exports of this crate's own dependencies, at the versions it was built with.
///
/// `armonik` and `armonik-transport-ffi` build against these instead of declaring their own
/// version requirements for the same crates, so nothing in the workspace can silently resolve to
/// two incompatible copies of `tonic`/`rustls`/... .
pub mod reexports {
    pub use hyper;
    pub use hyper_rustls;
    pub use hyper_util;
    pub use rustls;
    pub use tokio;
    pub use tonic;
    pub use tonic::codegen::http;
    pub use tonic::codegen::tokio_stream;
}
