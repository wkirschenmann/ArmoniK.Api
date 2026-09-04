use snafu::Snafu;

use crate::http2::TransportError;

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
        "`max_recv_message_size` of zero admits only empty messages, and zero is what a caller \
         means by `no limit`"
    ))]
    ZeroMaxRecvMessageSize,
    #[snafu(display("`{value}` is not a value a `user-agent` header can carry"))]
    InvalidUserAgent { value: String },
    #[snafu(display("{source}"), context(false))]
    Transport { source: TransportError },
}

#[derive(Clone, Debug, Eq, PartialEq, Snafu)]
#[non_exhaustive]
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
pub enum CallError {
    #[snafu(display("a message of {len} bytes does not fit the four-byte gRPC length prefix"))]
    MessageTooLong { len: usize },
    #[snafu(display("the call has already reached its terminal status"))]
    Ended,
    #[snafu(display("the task driving the call ended without a terminal status"))]
    Aborted,
}
