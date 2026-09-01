//! What the engine refuses, as opposed to what a call ends with.
//!
//! A call that reaches the network ends with a [`super::GrpcStatus`], whatever happened to it: a
//! connection that could not be made, a peer that answered something other than gRPC and a
//! cancellation all become a terminal status, because that is what gRPC promises a caller and
//! what the abstract model says every started call reaches. The errors here are the other
//! outcomes - the ones where no call is under way at all.

use super::metadata::MetadataError;

/// Why a call could not be started.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ChannelError {
    /// The channel is closed and takes no new calls.
    Closed,
    /// The method is not a path a gRPC request can carry.
    InvalidMethod {
        /// What was asked for.
        method: String,
    },
    /// The request metadata cannot become headers.
    InvalidMetadata {
        /// Which entry, and why.
        source: MetadataError,
    },
}

impl std::fmt::Display for ChannelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Closed => f.write_str("the channel is closed and takes no new calls"),
            Self::InvalidMethod { method } => write!(
                f,
                "`{method}` is not a method path; it has to be `/Service/Method`"
            ),
            Self::InvalidMetadata { source } => write!(f, "{source}"),
        }
    }
}

impl std::error::Error for ChannelError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidMetadata { source } => Some(source),
            _ => None,
        }
    }
}

/// Why an operation on a call could not happen.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CallError {
    /// The message is longer than the four-byte gRPC length prefix can express.
    MessageTooLong {
        /// How long it was.
        len: usize,
    },
    /// The call has reached its terminal, so there is nothing further to send or receive.
    Ended,
    /// The task driving the call went away without reaching a terminal, which is what an
    /// executor that drops its tasks looks like from here.
    Aborted,
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MessageTooLong { len } => write!(
                f,
                "a message of {len} bytes does not fit the four-byte gRPC length prefix"
            ),
            Self::Ended => f.write_str("the call has already reached its terminal status"),
            Self::Aborted => {
                f.write_str("the task driving the call ended without a terminal status")
            }
        }
    }
}

impl std::error::Error for CallError {}
