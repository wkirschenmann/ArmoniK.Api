//! Layer 2: the gRPC engine.
//!
//! Framing, metadata, status and calls, over the HTTP/2 session [`crate::http2`] establishes. It
//! speaks messages rather than bodies, which is what the FFI above it is built on.

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
