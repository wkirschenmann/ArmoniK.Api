
use crate::http2::TransportError;

use super::metadata::MetadataError;

#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ChannelError {
    Closed,
    Transport { source: TransportError },
    InvalidMethod { method: String },
    InvalidMetadata { source: MetadataError },
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
pub enum CallError {
    MessageTooLong { len: usize },
    Ended,
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
