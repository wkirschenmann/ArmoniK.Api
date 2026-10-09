//! Compression of a call's messages, as the gRPC compression document has it: the channel names
//! the encoding its calls send in and the ones it accepts in answers, a message carries a flag that
//! says whether it is compressed, and a peer may leave any message uncompressed.
//!
//! Sending is the engine's own, because the engine frames a message itself, below tonic's encoder;
//! receiving is tonic's decoder, which inflates within the channel's receive limit.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use flate2::write::{GzEncoder, ZlibEncoder};
use http::{HeaderMap, HeaderValue};
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

    /// The compressed form of `message`, or None when it is not smaller or the budget has no room
    /// for it: the flag is per message, so either goes out as it is.
    fn compress(
        self,
        message: &[u8],
        budget: Option<&dyn CompressionBudget>,
    ) -> Option<FramedMessage> {
        // Each encoder writes into a vector, which does not fail.
        let mut framed = match self {
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
        // Charged once made, at the size the allocation keeps, and given back when the message is
        // dropped, written or not. A copy refused is dropped: the one being built is outside the
        // budget.
        let charge = match budget {
            Some(budget) => {
                framed.shrink_to_fit();
                Some(budget.charge(framed.len())?)
            }
            None => None,
        };
        FramedMessage::compressed_in_place(framed, charge)
    }
}

/// What a message's compressed copy is counted against: the caller's memory ceiling, when it has
/// one. A copy it has no room for is dropped once made, and the message goes out as the caller
/// wrote it. The copy being built is outside the budget.
pub trait CompressionBudget: std::fmt::Debug + Send + Sync {
    /// Counts a copy of `bytes` bytes, or None when there is no room for it. Never waits. What it
    /// returns is dropped when the message holding the copy is, which gives the bytes back.
    fn charge(&self, bytes: usize) -> Option<Charge>;
}

/// Bytes counted against a [`CompressionBudget`], given back when it is dropped.
pub type Charge = Box<dyn Send + Sync>;

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

/// The header in which a server names the encodings it accepts.
const ACCEPT_ENCODING: &str = "grpc-accept-encoding";

/// Whether the `grpc-accept-encoding` of `headers` lists `encoding`; None when it has no such
/// header, which says nothing of what the server accepts. The names are compared without case, as
/// a list of tokens, and a value that is blank or not text is read as no header.
fn lists(headers: &HeaderMap, encoding: Encoding) -> Option<bool> {
    let mut seen = false;
    for value in headers.get_all(ACCEPT_ENCODING) {
        let Ok(text) = value.to_str() else { continue };
        if text.trim().is_empty() {
            continue;
        }
        seen = true;
        if text
            .split(',')
            .any(|name| name.trim().eq_ignore_ascii_case(encoding.name()))
        {
            return Some(true);
        }
    }
    seen.then_some(false)
}

/// The encoding a channel's calls send in, and what the server has said of accepting it.
///
/// A server states what it accepts in the `grpc-accept-encoding` of a response, and says it with
/// the `UNIMPLEMENTED` that refuses an encoding it cannot read. While the last such header leaves
/// the configured encoding out, calls that start send their messages as they are; a later one that
/// lists it, as a server that has been upgraded would, has them compressed once more. A response
/// without the header changes nothing. Behind a balancer whose backends differ, the state follows
/// whichever answered last.
///
/// The state is two flags, read without a lock on every call and written when a response changes
/// them. A call keeps what it was started with: the messages it has compressed and the header that
/// names their encoding have to agree.
pub(crate) struct SendEncoding {
    configured: Option<Encoding>,
    /// Whether the server's last word on what it accepts leaves the encoding out.
    refused: AtomicBool,
    /// Whether the channel has said so in a warning, which it does once.
    warned: AtomicBool,
}

impl SendEncoding {
    pub(crate) fn new(configured: Option<Encoding>) -> Self {
        Self {
            configured,
            refused: AtomicBool::new(false),
            warned: AtomicBool::new(false),
        }
    }

    /// What a call that starts now sends in.
    pub(crate) fn now(&self) -> Option<Encoding> {
        self.configured
            .filter(|_| !self.refused.load(Ordering::Relaxed))
    }

    /// Reads the head of a response, which may state what the server accepts.
    pub(crate) fn learn(&self, headers: &HeaderMap) {
        let Some(encoding) = self.configured else {
            return;
        };
        let Some(listed) = lists(headers, encoding) else {
            return;
        };
        // Nearly always the state already is what the head says, and nothing is written.
        let refused = !listed;
        if self.refused.load(Ordering::Relaxed) == refused {
            return;
        }
        self.refused.store(refused, Ordering::Relaxed);
        let name = encoding.name();
        if listed {
            tracing::debug!(
                target: "armonik_transport",
                encoding = name,
                "the server lists the encoding in grpc-accept-encoding: calls compress their messages again"
            );
        } else if !self.warned.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                target: "armonik_transport",
                encoding = name,
                accepted = %accepted_text(headers),
                "the server does not list the encoding in grpc-accept-encoding: calls send their messages uncompressed until a response lists it"
            );
        } else {
            tracing::debug!(
                target: "armonik_transport",
                encoding = name,
                "the server does not list the encoding in grpc-accept-encoding: calls send their messages uncompressed"
            );
        }
    }
}

/// What `headers` say in `grpc-accept-encoding`, for a log line.
fn accepted_text(headers: &HeaderMap) -> String {
    headers
        .get_all(ACCEPT_ENCODING)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect::<Vec<_>>()
        .join(";")
}

/// From this size a message is compressed on a blocking thread, where it does not hold up the
/// tasks of the runtime that carries the call: compressing a message of this size costs
/// milliseconds at these levels, and more for a larger one. Below it the work is short enough to
/// run in place.
const OFF_THE_RUNTIME_FROM: usize = 64 * 1024;

/// Whether `message` is compressed in place, rather than on a blocking thread.
pub(crate) fn compresses_in_place(message: &FramedMessage) -> bool {
    message.len() < OFF_THE_RUNTIME_FROM
}

/// `message` compressed with `encoding` where it is, or as it is when that gains nothing or the
/// budget has no room for the copy.
pub(crate) fn compressed_in_place(
    encoding: Encoding,
    message: FramedMessage,
    budget: Option<&dyn CompressionBudget>,
) -> FramedMessage {
    #[cfg(feature = "test-hooks")]
    crate::hooks::count_compression();
    if message.is_empty() {
        return message;
    }
    encoding
        .compress(message.payload(), budget)
        .unwrap_or(message)
}

/// `message` compressed with `encoding`, or as it is when that gains nothing or the budget has no
/// room for the copy.
pub(crate) async fn compressed(
    encoding: Encoding,
    message: FramedMessage,
    budget: Option<&Arc<dyn CompressionBudget>>,
) -> FramedMessage {
    if compresses_in_place(&message) {
        return compressed_in_place(encoding, message, budget.map(|budget| &**budget));
    }
    #[cfg(feature = "test-hooks")]
    crate::hooks::count_compression();
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return encoding
            .compress(message.payload(), budget.map(|budget| &**budget))
            .unwrap_or(message);
    };
    let whole: Bytes = message.body();
    let budget = budget.cloned();
    let done = runtime
        .spawn_blocking(move || encoding.compress(&whole[FRAME_PREFIX..], budget.as_deref()))
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

            let sent = compressed(encoding, message, None).await;

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

        let sent = compressed(Encoding::Deflate, message, None).await;

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

                let sent = compressed(encoding, message, None).await;

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

    fn head(values: &[&str]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for value in values {
            headers.append(
                ACCEPT_ENCODING,
                HeaderValue::from_str(value).expect("a value"),
            );
        }
        headers
    }

    #[test]
    fn a_server_lists_an_encoding_as_a_token_in_a_comma_separated_list() {
        let gzip = |values: &[&str]| lists(&head(values), Encoding::Gzip);

        assert_eq!(gzip(&[]), None, "no header says nothing");
        assert_eq!(gzip(&["gzip"]), Some(true));
        assert_eq!(gzip(&["identity,gzip"]), Some(true));
        assert_eq!(
            gzip(&["deflate, GZip ,zstd"]),
            Some(true),
            "spaces and case"
        );
        assert_eq!(
            gzip(&["identity", "gzip"]),
            Some(true),
            "a second header line"
        );
        assert_eq!(gzip(&["identity"]), Some(false));
        assert_eq!(
            gzip(&["gzipped,xgzip,gzip2"]),
            Some(false),
            "a token, not a substring"
        );
        assert_eq!(gzip(&[""]), None, "a blank value is no word on anything");
        assert_eq!(gzip(&[" ", "gzip"]), Some(true));

        let mut unreadable = HeaderMap::new();
        unreadable.insert(
            ACCEPT_ENCODING,
            HeaderValue::from_bytes(b"gz\xffip").expect("opaque bytes"),
        );
        assert_eq!(lists(&unreadable, Encoding::Gzip), None);
    }

    #[test]
    fn what_a_channel_sends_in_follows_what_the_server_lists() {
        let send = SendEncoding::new(Some(Encoding::Zstd));
        assert_eq!(send.now(), Some(Encoding::Zstd));

        send.learn(&head(&[]));
        send.learn(&head(&["zstd,identity"]));
        assert_eq!(send.now(), Some(Encoding::Zstd), "nothing says otherwise");

        send.learn(&head(&["gzip,identity"]));
        assert_eq!(send.now(), None, "the server does not list it");
        send.learn(&head(&[]));
        assert_eq!(
            send.now(),
            None,
            "a head without the header changes nothing"
        );

        send.learn(&head(&["gzip,zstd,identity"]));
        assert_eq!(send.now(), Some(Encoding::Zstd), "it is listed again");
    }

    #[test]
    fn a_channel_that_sends_nothing_has_nothing_to_learn() {
        let send = SendEncoding::new(None);

        send.learn(&head(&["identity"]));

        assert_eq!(send.now(), None);
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

            let sent = compressed(encoding, message, None).await;

            assert_eq!(sent.body()[0], 1);
            assert_eq!(inflated(encoding, &sent), text, "{encoding:?}");
        }
    }

    /// Outside a runtime the same future runs to its end by a plain poll.
    #[test]
    fn a_large_message_is_compressed_where_there_is_no_runtime() {
        let text = vec![7; 2 * OFF_THE_RUNTIME_FROM];
        let message = FramedMessage::copy_of(&text).expect("a message");

        let sent = ready_now(compressed(Encoding::Zstd, message, None));

        assert_eq!(sent.body()[0], 1);
        assert_eq!(inflated(Encoding::Zstd, &sent), text);
    }

    /// A budget of `room` bytes that counts what it has charged.
    #[derive(Debug)]
    struct Room {
        room: usize,
        charged: Arc<std::sync::atomic::AtomicUsize>,
    }

    struct Counted(usize, Arc<std::sync::atomic::AtomicUsize>);

    impl Drop for Counted {
        fn drop(&mut self) {
            self.1.fetch_sub(self.0, Ordering::SeqCst);
        }
    }

    impl CompressionBudget for Room {
        fn charge(&self, bytes: usize) -> Option<Charge> {
            if self.charged.load(Ordering::SeqCst) + bytes > self.room {
                return None;
            }
            self.charged.fetch_add(bytes, Ordering::SeqCst);
            Some(Box::new(Counted(bytes, Arc::clone(&self.charged))))
        }
    }

    /// The copy is counted while the message lives and given back with it; a copy the budget has
    /// no room for is dropped, and the message goes as it is, through either path.
    #[tokio::test]
    async fn a_copy_is_charged_for_the_life_of_its_message_and_refused_when_it_does_not_fit() {
        for encoding in ALL {
            for len in [10_000, 4 * OFF_THE_RUNTIME_FROM] {
                let text = vec![7; len];
                let charged = Arc::new(std::sync::atomic::AtomicUsize::new(0));
                let budget: Arc<dyn CompressionBudget> = Arc::new(Room {
                    room: 1000,
                    charged: Arc::clone(&charged),
                });

                let sent = compressed(
                    encoding,
                    FramedMessage::copy_of(&text).expect("a message"),
                    Some(&budget),
                )
                .await;
                assert_eq!(sent.body()[0], 1, "{encoding:?} {len}");
                let held = charged.load(Ordering::SeqCst);
                assert_eq!(held, sent.body().len(), "{encoding:?} {len}");
                assert!(held > 0 && held <= 1000, "{encoding:?} {len}: {held}");
                let copy = sent.body();
                drop(sent);
                assert_eq!(
                    charged.load(Ordering::SeqCst),
                    held,
                    "a copy still holds it"
                );
                drop(copy);
                assert_eq!(charged.load(Ordering::SeqCst), 0, "{encoding:?} {len}");

                let none: Arc<dyn CompressionBudget> = Arc::new(Room {
                    room: held - 1,
                    charged: Arc::clone(&charged),
                });
                let whole = FramedMessage::copy_of(&text).expect("a message");
                let before = whole.body();
                let sent = compressed(encoding, whole, Some(&none)).await;
                assert_eq!(sent.body(), before, "{encoding:?} {len}: as it was");
                assert_eq!(sent.body()[0], 0);
                assert_eq!(charged.load(Ordering::SeqCst), 0, "{encoding:?} {len}");
            }
        }
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
