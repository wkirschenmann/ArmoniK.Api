//! What the engine refuses, as opposed to what a call ends with.
//!
//! A call that reaches the network ends with a [`super::GrpcStatus`], whatever happens to it: a
//! connection that cannot be made, a peer that answers something other than gRPC, a cancellation.
//! That is what gRPC promises a caller, and what the abstract model means by every started call
//! reaching a terminal. The errors here are the other outcomes, where no call is under way.

use snafu::Snafu;

use crate::http2::TransportError;

use super::metadata::MetadataError;

#[derive(Clone, Debug, Eq, PartialEq, Snafu)]
#[non_exhaustive]
/// Why a channel could not be built.
///
/// Distinct from [`ChannelError`], which is why a channel that exists could not do something, and
/// distinct from [`crate::config::ConfigError`], which is the environment-driven client's: these
/// are the engine's refusals, and a caller reading one has no reason to meet a vocabulary about
/// `GrpcClient__CertPem` and PEM parsing on the way to it.
///
/// Every variant is decided before any I/O, so none of them says the endpoint was unreachable.
pub enum GrpcChannelConfigError {
    #[snafu(display(
        "`max_sends_in_flight` is the number of buffers a call may have out at once, so zero \
         would let it send nothing"
    ))]
    ZeroSendWindow,
    #[snafu(display(
        "`max_recv_message_size` of zero admits only empty messages, and zero is what a caller \
         means by `no limit`"
    ))]
    ZeroMaxRecvMessageSize,
    #[snafu(display("`{value}` is not a value a `user-agent` header can carry"))]
    InvalidUserAgent { value: String },
    /// An endpoint no connector could dial.
    #[snafu(display("{source}"), context(false))]
    Transport { source: TransportError },
}

#[derive(Clone, Debug, Eq, PartialEq, Snafu)]
#[non_exhaustive]
/// Why a channel could not do what was asked of it.
pub enum ChannelError {
    #[snafu(display("the channel is closed and takes no new calls"))]
    Closed,
    #[snafu(display("{source}"), context(false))]
    Transport { source: TransportError },
    #[snafu(display("`{method}` is not a method path; it has to be `/Service/Method`"))]
    InvalidMethod { method: String },
    #[snafu(display("{source}"))]
    InvalidMetadata { source: MetadataError },
}

#[derive(Clone, Debug, Eq, PartialEq, Snafu)]
#[non_exhaustive]
/// Why an operation on a call could not happen.
pub enum CallError {
    #[snafu(display("a message of {len} bytes does not fit the four-byte gRPC length prefix"))]
    MessageTooLong { len: usize },
    #[snafu(display("the call has already reached its terminal status"))]
    Ended,
    /// The task driving the call went away without reaching a terminal, which is what an
    /// executor that drops its tasks looks like from here.
    #[snafu(display("the task driving the call ended without a terminal status"))]
    Aborted,
}
