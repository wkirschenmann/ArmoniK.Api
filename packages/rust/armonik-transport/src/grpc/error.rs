//! What the engine refuses, as opposed to what a call ends with.
//!
//! A call that reaches the network ends with a [`super::GrpcStatus`], whatever happens to it: a
//! connection that cannot be made, a peer that answers something other than gRPC, a cancellation.
//! That is what gRPC promises a caller, and what the abstract model means by every started call
//! reaching a terminal. The errors here are the other outcomes, where no call is under way.

use crate::http2::TransportError;

use super::metadata::MetadataError;

#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
/// Why a channel could not do what was asked of it.
pub enum ChannelError {
    /// The channel is closed and takes no new calls.
    Closed,
    Transport {
        source: TransportError,
    },
    InvalidMethod {
        method: String,
    },
    InvalidMetadata {
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
            Self::Transport { source } => write!(f, "{source}"),
        }
    }
}

impl From<TransportError> for ChannelError {
    fn from(source: TransportError) -> Self {
        Self::Transport { source }
    }
}

impl std::error::Error for ChannelError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidMetadata { source } => Some(source),
            Self::Transport { source } => Some(source),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
/// Why an operation on a call could not happen.
pub enum CallError {
    /// The message is longer than the four-byte gRPC length prefix can express.
    MessageTooLong { len: usize },
    /// The call has reached its terminal; the status it ended with is on the reading half.
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
