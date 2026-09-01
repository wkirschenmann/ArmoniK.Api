//! Layer 2: the gRPC engine.
//!
//! Framing, metadata, status and calls, over the HTTP/2 session the [`crate::http2`] connector's
//! stream carries. It speaks messages rather than bodies, which is what the FFI is built on.

mod call;
mod channel;
mod error;
mod executor;
mod frame;
mod metadata;
mod status;

pub use call::{
    CallControl, CallStartOptions, GrpcCall, OwnedMessage, RecvHalf, RecvResult, SendHalf,
};
pub use channel::{GrpcChannel, GrpcChannelConfig};
pub use error::{CallError, ChannelError};
pub use executor::{BoxedTask, Executor, TaskHandle, TokioExecutor};
pub use metadata::{Metadata, MetadataError, MetadataValue};
pub use status::{GrpcStatus, GrpcStatusCode};
