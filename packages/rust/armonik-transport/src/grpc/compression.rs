//! Compression of a call's messages, as the gRPC compression document has it: the channel names
//! the encoding its calls send in and the one it accepts in answers, a message carries a flag that
//! says whether it is compressed, and a peer may leave any message uncompressed.
//!
//! Sending is the engine's own, because the engine frames a message itself, below tonic's encoder;
//! receiving is tonic's decoder, which inflates within the channel's receive limit.

use std::io::Write;

use bytes::Bytes;
use flate2::write::GzEncoder;
use tonic::codec::CompressionEncoding;

use super::request::{FramedMessage, FRAME_PREFIX};

/// A message encoding, by the name `grpc-encoding` and `grpc-accept-encoding` give it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Encoding {
    /// RFC 1952 gzip, at the compression level zlib calls default.
    Gzip,
}

impl Encoding {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Gzip => "gzip",
        }
    }

    /// What `grpc-accept-encoding` says of a channel that accepts this encoding.
    pub(crate) fn accepted(self) -> &'static str {
        match self {
            Self::Gzip => "gzip,identity",
        }
    }

    pub(crate) fn for_tonic(self) -> CompressionEncoding {
        match self {
            Self::Gzip => CompressionEncoding::Gzip,
        }
    }

    /// The compressed form of `message`, or None when it is not smaller: the flag is per message,
    /// so one that gains nothing goes out as it is.
    fn compress(self, message: &[u8]) -> Option<FramedMessage> {
        let mut framed = Vec::with_capacity(FRAME_PREFIX + message.len() / 2 + 32);
        framed.resize(FRAME_PREFIX, 0);
        let framed = match self {
            Self::Gzip => {
                let mut encoder = GzEncoder::new(framed, flate2::Compression::default());
                // Into a vector, which does not fail.
                encoder.write_all(message).ok()?;
                encoder.finish().ok()?
            }
        };
        if framed.len() - FRAME_PREFIX >= message.len() {
            return None;
        }
        FramedMessage::compressed_in_place(framed)
    }
}

/// From this size a message is compressed on a blocking thread, where it does not hold up the
/// tasks of the runtime that carries the call: gzip at this level costs milliseconds on a message
/// of this size and more on a larger one. Below it the work is short enough to run in place.
const OFF_THE_RUNTIME_FROM: usize = 64 * 1024;

/// `message` compressed with `encoding`, or as it is when that gains nothing.
pub(crate) async fn compressed(encoding: Encoding, message: FramedMessage) -> FramedMessage {
    if message.is_empty() {
        return message;
    }
    if message.len() < OFF_THE_RUNTIME_FROM {
        return encoding.compress(message.payload()).unwrap_or(message);
    }
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return encoding.compress(message.payload()).unwrap_or(message);
    };
    let whole: Bytes = message.body();
    let done = runtime
        .spawn_blocking(move || encoding.compress(&whole[FRAME_PREFIX..]))
        .await;
    // A task that did not finish leaves the message as it was, which is a valid answer.
    match done {
        Ok(compressed) => compressed.unwrap_or(message),
        Err(error) => {
            tracing::warn!(%error, "a message goes uncompressed: its compression did not finish");
            message
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use flate2::read::GzDecoder;

    use super::*;

    fn inflated(message: &FramedMessage) -> Vec<u8> {
        let mut out = Vec::new();
        GzDecoder::new(message.payload())
            .read_to_end(&mut out)
            .expect("gzip");
        out
    }

    #[tokio::test]
    async fn a_message_that_compresses_is_flagged_and_inflates_to_itself() {
        let text = b"abc".repeat(1000);
        let message = FramedMessage::copy_of(&text).expect("a message");

        let sent = compressed(Encoding::Gzip, message).await;

        assert_eq!(sent.body()[0], 1, "the compressed flag");
        assert_eq!(
            u32::from_be_bytes(sent.body()[1..FRAME_PREFIX].try_into().expect("four bytes")),
            sent.len() as u32
        );
        assert!(sent.len() < text.len());
        assert_eq!(inflated(&sent), text);
    }

    #[tokio::test]
    async fn a_message_that_gains_nothing_goes_as_it_is() {
        for text in [&b""[..], b"x", b"a few bytes of no pattern"] {
            let message = FramedMessage::copy_of(text).expect("a message");
            let before = message.body();

            let sent = compressed(Encoding::Gzip, message).await;

            assert_eq!(sent.body(), before, "{text:?}");
            assert_eq!(sent.body()[0], 0);
        }
    }

    /// Above the size where it leaves the runtime's tasks, the message comes back as compressed as
    /// one compressed in place.
    #[tokio::test]
    async fn a_large_message_is_compressed_the_same_through_the_blocking_pool() {
        let text = vec![7; 4 * OFF_THE_RUNTIME_FROM];
        let message = FramedMessage::copy_of(&text).expect("a message");

        let sent = compressed(Encoding::Gzip, message).await;

        assert_eq!(sent.body()[0], 1);
        assert_eq!(inflated(&sent), text);
    }

    /// Outside a runtime the same future runs to its end by a plain poll.
    #[test]
    fn a_large_message_is_compressed_where_there_is_no_runtime() {
        let text = vec![7; 2 * OFF_THE_RUNTIME_FROM];
        let message = FramedMessage::copy_of(&text).expect("a message");

        let sent = ready_now(compressed(Encoding::Gzip, message));

        assert_eq!(sent.body()[0], 1);
        assert_eq!(inflated(&sent), text);
    }

    /// Polls a future that never waits.
    fn ready_now<T>(future: impl std::future::Future<Output = T>) -> T {
        let mut future = std::pin::pin!(future);
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        match future.as_mut().poll(&mut cx) {
            std::task::Poll::Ready(value) => value,
            std::task::Poll::Pending => panic!("it waits for nothing"),
        }
    }
}
