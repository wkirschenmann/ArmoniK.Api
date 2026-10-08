use snafu::Snafu;

use crate::http2::{TransportError, TransportErrorKind};
use crate::options::LARGEST_WINDOW;

use super::metadata::MetadataError;

#[derive(Clone, Debug, Eq, PartialEq, Snafu)]
#[non_exhaustive]
pub enum GrpcChannelConfigError {
    #[snafu(display(
        "`max_sends_in_flight` is the number of buffers a call may have out at once, so zero \
         would let it send nothing"
    ))]
    ZeroSendWindow,
    #[snafu(display(
        "a `max_sends_in_flight` of {value} is past {LARGEST_WINDOW}, the deepest window the \
         options admit"
    ))]
    SendWindowTooLarge { value: usize },
    #[snafu(display(
        "`max_recv_message_size` of zero admits only empty messages, and zero is what a caller \
         means by `no limit`"
    ))]
    ZeroMaxRecvMessageSize,
    #[snafu(display(
        "`max_send_message_size` of zero admits only empty messages, and no limit is `None`"
    ))]
    ZeroMaxSendMessageSize,
    #[snafu(display("`{value}` is not a value a `user-agent` header can carry"))]
    InvalidUserAgent { value: String },
    #[snafu(display("the retry policy is refused: {why}"))]
    Retry { why: String },
    #[snafu(display("the adaptive rate is refused: {why}"))]
    Adaptive { why: String },
    #[snafu(display("{source}"), context(false))]
    Transport { source: TransportError },
}

#[derive(Clone, Debug, Eq, PartialEq, Snafu)]
#[non_exhaustive]
pub enum ChannelError {
    #[snafu(display("the channel is closed and takes no new calls"))]
    Closed,
    #[snafu(display("the engine panicked while connecting to `{endpoint}`"))]
    DialPanicked { endpoint: String },
    #[snafu(display("{source}"), context(false))]
    Transport { source: TransportError },
    #[snafu(display("`{method}` is not a method path; it has to be `/Service/Method`"))]
    InvalidMethod { method: String },
    #[snafu(display("{source}"))]
    InvalidMetadata { source: MetadataError },
}

impl ChannelError {
    /// Whether a later dial may succeed where this one failed: the peer or the network is out of
    /// reach, or a handshake fails, which a server that is restarted or reconfigured cures. A
    /// closed channel, the engine's panic and a configuration no dial can satisfy are not.
    pub(crate) fn is_connection_failure(&self) -> bool {
        match self {
            Self::Transport { source } => source.kind() != TransportErrorKind::Configuration,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Snafu)]
#[non_exhaustive]
pub enum CallError {
    #[snafu(display("a message of {len} bytes does not fit the four-byte gRPC length prefix"))]
    MessageTooLong { len: usize },
    #[snafu(display(
        "a message of {len} bytes is past the {max} the channel sends; the call ends \
         RESOURCE_EXHAUSTED"
    ))]
    MessageTooLarge { len: usize, max: usize },
    #[snafu(display("the call has already reached its terminal status"))]
    Ended,
    #[snafu(display("the task driving the call ended without a terminal status"))]
    Aborted,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transport(source: TransportError) -> ChannelError {
        ChannelError::Transport { source }
    }

    #[test]
    fn a_dial_that_a_later_dial_may_cure_is_a_connection_failure() {
        let endpoint = "http://h:1".to_owned();
        let cause = "no".to_owned();
        for source in [
            TransportError::Connect {
                endpoint: endpoint.clone(),
                cause: cause.clone(),
            },
            TransportError::TlsHandshake {
                endpoint: endpoint.clone(),
                cause: cause.clone(),
            },
            TransportError::Timeout {
                endpoint,
                after: std::time::Duration::from_secs(1),
            },
        ] {
            assert!(
                transport(source.clone()).is_connection_failure(),
                "{source}"
            );
        }
    }

    #[test]
    fn a_closed_channel_a_panic_and_a_configuration_are_not() {
        assert!(!ChannelError::Closed.is_connection_failure());
        assert!(!ChannelError::DialPanicked {
            endpoint: "http://h:1".to_owned()
        }
        .is_connection_failure());
        assert!(!transport(TransportError::Configuration {
            message: "no".to_owned()
        })
        .is_connection_failure());
    }
}
