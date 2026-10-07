//! Compression of a call's messages, as the gRPC compression document has it: the channel names
//! the encoding its calls send in and the ones it accepts in answers, a message carries a flag that
//! says whether it is compressed, and a peer may leave any message uncompressed.
//!
//! Sending is the engine's own, because the engine frames a message itself, below tonic's encoder;
//! receiving is tonic's decoder, which inflates within the channel's receive limit.

use std::io::Write;

use bytes::Bytes;
use flate2::write::{GzEncoder, ZlibEncoder};
use http::HeaderValue;
use tonic::codec::CompressionEncoding;

use super::request::{FramedMessage, FRAME_PREFIX};

/// A message encoding, by the name `grpc-encoding` and `grpc-accept-encoding` give it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Encoding {
    /// RFC 1952 gzip, at the compression level zlib calls default.
    Gzip,
    /// gRPC's `deflate`: the zlib structure of RFC 1950 around an RFC 1951 stream, as zlib's
    /// `deflate` produces and `inflate` reads, at the default level. It is not a raw RFC 1951 stream.
    Deflate,
    /// RFC 8878 Zstandard, at the library's default level.
    Zstd,
}

impl Encoding {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Gzip => "gzip",
            Self::Deflate => "deflate",
            Self::Zstd => "zstd",
        }
    }

    pub(crate) fn for_tonic(self) -> CompressionEncoding {
        match self {
            Self::Gzip => CompressionEncoding::Gzip,
            Self::Deflate => CompressionEncoding::Deflate,
            Self::Zstd => CompressionEncoding::Zstd,
        }
    }

    /// A buffer for the compressed form of `message`, [`FRAME_PREFIX`] bytes kept for the prefix.
    fn prefixed(message: &[u8]) -> Vec<u8> {
        let mut framed = Vec::with_capacity(FRAME_PREFIX + message.len() / 2 + 32);
        framed.resize(FRAME_PREFIX, 0);
        framed
    }

    /// The compressed form of `message`, or None when it is not smaller: the flag is per message,
    /// so one that gains nothing goes out as it is.
    fn compress(self, message: &[u8]) -> Option<FramedMessage> {
        // Each encoder writes into a vector, which does not fail.
        let framed = match self {
            Self::Gzip => {
                let mut encoder =
                    GzEncoder::new(Self::prefixed(message), flate2::Compression::default());
                encoder.write_all(message).ok()?;
                encoder.finish().ok()?
            }
            Self::Deflate => {
                let mut encoder =
                    ZlibEncoder::new(Self::prefixed(message), flate2::Compression::default());
                encoder.write_all(message).ok()?;
                encoder.finish().ok()?
            }
            // One call that knows the message's size, so that the library sizes its context to
            // the message and not to the level's largest window.
            Self::Zstd => {
                let bound = zstd::zstd_safe::compress_bound(message.len());
                let mut framed = vec![0; FRAME_PREFIX + bound];
                let written = zstd::bulk::Compressor::new(zstd::DEFAULT_COMPRESSION_LEVEL)
                    .ok()?
                    .compress_to_buffer(message, &mut framed[FRAME_PREFIX..])
                    .ok()?;
                framed.truncate(FRAME_PREFIX + written);
                framed
            }
        };
        if framed.len() - FRAME_PREFIX >= message.len() {
            return None;
        }
        FramedMessage::compressed_in_place(framed)
    }
}

/// `encodings` with a repeated one left out after its first, in the order given.
pub(crate) fn distinct(encodings: &[Encoding]) -> Vec<Encoding> {
    let mut kept: Vec<Encoding> = Vec::with_capacity(encodings.len());
    for encoding in encodings {
        if !kept.contains(encoding) {
            kept.push(*encoding);
        }
    }
    kept
}

/// What `grpc-accept-encoding` says of a channel that accepts `accepted`: their names in order,
/// which a server that picks the first it knows reads as a preference, and `identity` last, which
/// is always accepted. With none, only `identity`: a peer that reads it then sends what can be
/// read rather than a body that cannot.
pub(crate) fn accept_header(accepted: &[Encoding]) -> HeaderValue {
    let mut names: Vec<&str> = accepted.iter().map(|encoding| encoding.name()).collect();
    names.push("identity");
    HeaderValue::from_str(&names.join(",")).expect("encoding names are ASCII tokens")
}

/// From this size a message is compressed on a blocking thread, where it does not hold up the
/// tasks of the runtime that carries the call: compressing a message of this size costs
/// milliseconds at these levels, and more for a larger one. Below it the work is short enough to
/// run in place.
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

    use flate2::read::{GzDecoder, ZlibDecoder};

    use super::*;

    const ALL: [Encoding; 3] = [Encoding::Gzip, Encoding::Deflate, Encoding::Zstd];

    /// `message` inflated by a decoder of `encoding` that is not the encoder's own.
    fn inflated(encoding: Encoding, message: &FramedMessage) -> Vec<u8> {
        let mut out = Vec::new();
        let payload = message.payload();
        match encoding {
            Encoding::Gzip => GzDecoder::new(payload).read_to_end(&mut out),
            Encoding::Deflate => ZlibDecoder::new(payload).read_to_end(&mut out),
            Encoding::Zstd => zstd::stream::read::Decoder::new(payload)
                .expect("a zstd frame")
                .read_to_end(&mut out),
        }
        .expect("a stream of the encoding");
        out
    }

    #[tokio::test]
    async fn a_message_that_compresses_is_flagged_and_inflates_to_itself() {
        for encoding in ALL {
            let text = b"abc".repeat(1000);
            let message = FramedMessage::copy_of(&text).expect("a message");

            let sent = compressed(encoding, message).await;

            assert_eq!(sent.body()[0], 1, "the compressed flag, {encoding:?}");
            assert_eq!(
                u32::from_be_bytes(sent.body()[1..FRAME_PREFIX].try_into().expect("four bytes")),
                sent.len() as u32
            );
            assert!(sent.len() < text.len());
            assert_eq!(inflated(encoding, &sent), text, "{encoding:?}");
        }
    }

    /// gRPC's deflate is zlib's structure: a two-byte header whose low nibble is 8 and which, read
    /// as a big-endian number, is a multiple of 31 (RFC 1950).
    #[tokio::test]
    async fn deflate_is_the_zlib_structure() {
        let message = FramedMessage::copy_of(&b"abc".repeat(1000)).expect("a message");

        let sent = compressed(Encoding::Deflate, message).await;

        let header = &sent.payload()[..2];
        assert_eq!(header[0] & 0x0f, 8, "the deflate method");
        assert_eq!(u16::from_be_bytes([header[0], header[1]]) % 31, 0);
    }

    #[tokio::test]
    async fn a_message_that_gains_nothing_goes_as_it_is() {
        for encoding in ALL {
            for text in [&b""[..], b"x", b"a few bytes of no pattern"] {
                let message = FramedMessage::copy_of(text).expect("a message");
                let before = message.body();

                let sent = compressed(encoding, message).await;

                assert_eq!(sent.body(), before, "{encoding:?} {text:?}");
                assert_eq!(sent.body()[0], 0);
            }
        }
    }

    #[test]
    fn the_names_are_the_ones_the_document_gives_and_the_header_lists_them_in_order() {
        assert_eq!(accept_header(&[]), "identity");
        assert_eq!(accept_header(&[Encoding::Gzip]), "gzip,identity");
        assert_eq!(
            accept_header(&[Encoding::Zstd, Encoding::Deflate, Encoding::Gzip]),
            "zstd,deflate,gzip,identity"
        );
        assert_eq!(
            ALL.map(Encoding::name),
            ["gzip", "deflate", "zstd"],
            "tonic compares these names exactly"
        );
    }

    #[test]
    fn a_repeated_encoding_is_kept_at_its_first_place() {
        use Encoding::*;
        assert_eq!(
            distinct(&[Zstd, Gzip, Zstd, Gzip, Deflate, Gzip]),
            [Zstd, Gzip, Deflate]
        );
        assert!(distinct(&[]).is_empty());
    }

    /// Above the size where it leaves the runtime's tasks, the message comes back as compressed as
    /// one compressed in place.
    #[tokio::test]
    async fn a_large_message_is_compressed_the_same_through_the_blocking_pool() {
        for encoding in ALL {
            let text = vec![7; 4 * OFF_THE_RUNTIME_FROM];
            let message = FramedMessage::copy_of(&text).expect("a message");

            let sent = compressed(encoding, message).await;

            assert_eq!(sent.body()[0], 1);
            assert_eq!(inflated(encoding, &sent), text, "{encoding:?}");
        }
    }

    /// Outside a runtime the same future runs to its end by a plain poll.
    #[test]
    fn a_large_message_is_compressed_where_there_is_no_runtime() {
        let text = vec![7; 2 * OFF_THE_RUNTIME_FROM];
        let message = FramedMessage::copy_of(&text).expect("a message");

        let sent = ready_now(compressed(Encoding::Zstd, message));

        assert_eq!(sent.body()[0], 1);
        assert_eq!(inflated(Encoding::Zstd, &sent), text);
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
