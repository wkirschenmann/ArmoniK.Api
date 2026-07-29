//! Rust bindings for the ArmoniK API

pub mod api;
#[cfg(feature = "_gen-client")]
pub mod client;
mod objects;
#[cfg(feature = "_gen-server")]
pub mod server;

#[cfg(feature = "_gen-client")]
pub use client::{Client, ClientConfig, ProxyConfig, ProxySource, RetryPolicy};
pub use objects::*;

mod utils;

pub mod reexports {
    pub use hyper;
    // `hyper-rustls`/`rustls` moved to `armonik-transport` along with the rest of the connection
    // stack; re-exported here so `armonik::reexports::{hyper_rustls, rustls}` keeps resolving.
    #[cfg(feature = "_gen-client")]
    pub use armonik_transport::reexports::{hyper_rustls, rustls};
    pub use prost;
    pub use prost_types;
    #[cfg(feature = "serde")]
    pub use serde;
    #[cfg(feature = "_gen-server")]
    pub use tokio;
    pub use tonic;
    pub use tonic::async_trait;
    pub use tonic::codegen::http;
    pub use tonic::codegen::tokio_stream;
    pub use tracing;
    pub use tracing_futures;
}
