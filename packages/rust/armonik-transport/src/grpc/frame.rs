//! The gRPC length-prefixed framing: one flag byte, a four-byte length, then the message.

use std::collections::VecDeque;

use bytes::buf::Chain;
use bytes::{Buf, Bytes, BytesMut};

use super::error::CallError;

/// The flag byte plus the four length bytes that precede every message.
const HEADER_LEN: usize = 5;

/// The only compression flag this engine sends or accepts.
const UNCOMPRESSED: u8 = 0;

/// Frames a message for the wire.
///
/// The payload is chained behind its header rather than copied into one buffer: the caller's
/// allocation is what goes out, which is what makes a send across the FFI free of a copy.
pub(crate) fn frame(payload: Bytes) -> Result<Chain<Bytes, Bytes>, CallError> {
    let len = u32::try_from(payload.len()).map_err(|_| CallError::MessageTooLong {
        len: payload.len(),
    })?;

    let mut header = BytesMut::with_capacity(HEADER_LEN);
    header.extend_from_slice(&[UNCOMPRESSED]);
    header.extend_from_slice(&len.to_be_bytes());

    Ok(header.freeze().chain(payload))
}

/// Reassembles messages from the chunks a response body hands over.
///
/// Chunks are kept as they arrive and consumed in place, so a message that fits in one chunk is a
/// view on that chunk rather than a copy of it. Only a message spanning several chunks is
/// assembled into a buffer of its own.
#[derive(Debug, Default)]
pub(crate) struct Deframer {
    chunks: VecDeque<Bytes>,
    buffered: usize,
}

impl Deframer {
    /// Takes a chunk of the response body.
    pub(crate) fn push(&mut self, chunk: Bytes) {
        if chunk.is_empty() {
            return;
        }
        self.buffered += chunk.len();
        self.chunks.push_back(chunk);
    }

    /// Whether everything pushed so far has been read back out as whole messages.
    pub(crate) fn is_at_message_boundary(&self) -> bool {
        self.buffered == 0
    }

    /// The next whole message, if one has arrived.
    pub(crate) fn next_message(&mut self) -> Result<Option<Bytes>, DeframeError> {
        if self.buffered < HEADER_LEN {
            return Ok(None);
        }

        let mut header = [0u8; HEADER_LEN];
        self.peek(&mut header);

        match header[0] {
            UNCOMPRESSED => {}
            1 => return Err(DeframeError::Compressed),
            flag => return Err(DeframeError::UnknownFlag { flag }),
        }

        let len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
        if self.buffered < HEADER_LEN + len {
            return Ok(None);
        }

        self.advance(HEADER_LEN);
        Ok(Some(self.take(len)))
    }

    /// Copies the first `out.len()` bytes without consuming them. The caller has checked there
    /// are that many.
    fn peek(&self, out: &mut [u8]) {
        let mut filled = 0;
        for chunk in &self.chunks {
            let take = (out.len() - filled).min(chunk.len());
            out[filled..filled + take].copy_from_slice(&chunk[..take]);
            filled += take;
            if filled == out.len() {
                return;
            }
        }
    }

    /// Drops `count` bytes. The caller has checked there are that many.
    fn advance(&mut self, mut count: usize) {
        self.buffered -= count;
        while count > 0 {
            let head = self.chunks.front_mut().expect("the bytes were counted");
            let take = count.min(head.len());
            head.advance(take);
            count -= take;
            if head.is_empty() {
                self.chunks.pop_front();
            }
        }
    }

    /// Consumes `count` bytes as one message. The caller has checked there are that many.
    fn take(&mut self, count: usize) -> Bytes {
        // An empty message consumes no chunk, and there may be none left to reach for.
        if count == 0 {
            return Bytes::new();
        }
        self.buffered -= count;

        let head = self.chunks.front_mut().expect("the bytes were counted");
        if head.len() >= count {
            let message = head.split_to(count);
            if head.is_empty() {
                self.chunks.pop_front();
            }
            return message;
        }

        let mut message = BytesMut::with_capacity(count);
        while message.len() < count {
            let head = self.chunks.front_mut().expect("the bytes were counted");
            let take = (count - message.len()).min(head.len());
            message.extend_from_slice(&head[..take]);
            head.advance(take);
            if head.is_empty() {
                self.chunks.pop_front();
            }
        }
        message.freeze()
    }
}

/// What a peer's stream of frames can get wrong.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DeframeError {
    /// The peer compressed a message, having been told this engine accepts none.
    Compressed,
    /// A flag byte that is neither compressed nor uncompressed.
    UnknownFlag { flag: u8 },
}

impl std::fmt::Display for DeframeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Compressed => f.write_str(
                "the peer sent a compressed message, though this channel advertises `identity` \
                 alone",
            ),
            Self::UnknownFlag { flag } => write!(
                f,
                "a message carried the compression flag {flag}, which gRPC does not define"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn framed(payload: &'static [u8]) -> Bytes {
        let mut chained = frame(Bytes::from_static(payload)).expect("short enough");
        chained.copy_to_bytes(chained.remaining())
    }

    fn drain(deframer: &mut Deframer) -> Vec<Bytes> {
        let mut out = Vec::new();
        while let Some(message) = deframer.next_message().expect("well-formed frames") {
            out.push(message);
        }
        out
    }

    #[test]
    fn a_message_is_its_length_in_big_endian_behind_a_zero_flag() {
        assert_eq!(&framed(b"hi")[..], &[0, 0, 0, 0, 2, b'h', b'i']);
        assert_eq!(&framed(b"")[..], &[0, 0, 0, 0, 0]);
    }

    #[test]
    fn a_message_that_fits_one_chunk_is_a_view_on_that_chunk() {
        let chunk = framed(b"payload");
        let mut deframer = Deframer::default();
        deframer.push(chunk.clone());

        let message = deframer
            .next_message()
            .expect("well-formed")
            .expect("a whole message");
        assert_eq!(&message[..], b"payload");
        // The same allocation, sliced: the pointer sits inside the chunk that was pushed.
        assert_eq!(
            message.as_ptr() as usize,
            chunk.as_ptr() as usize + HEADER_LEN
        );
        assert!(deframer.is_at_message_boundary());
    }

    #[test]
    fn a_message_split_across_chunks_is_reassembled() {
        let whole = framed(b"across the chunks");
        let mut deframer = Deframer::default();
        for byte in whole.iter() {
            assert_eq!(deframer.next_message().expect("well-formed"), None);
            deframer.push(Bytes::copy_from_slice(&[*byte]));
        }

        assert_eq!(drain(&mut deframer), vec![Bytes::from_static(b"across the chunks")]);
        assert!(deframer.is_at_message_boundary());
    }

    #[test]
    fn several_messages_in_one_chunk_come_out_in_order() {
        let mut joined = BytesMut::new();
        joined.extend_from_slice(&framed(b"one"));
        joined.extend_from_slice(&framed(b""));
        joined.extend_from_slice(&framed(b"three"));

        let mut deframer = Deframer::default();
        deframer.push(joined.freeze());

        assert_eq!(
            drain(&mut deframer),
            vec![
                Bytes::from_static(b"one"),
                Bytes::from_static(b""),
                Bytes::from_static(b"three"),
            ]
        );
        assert!(deframer.is_at_message_boundary());
    }

    #[test]
    fn a_stream_that_ends_mid_message_is_not_at_a_boundary() {
        let whole = framed(b"truncated");
        let mut deframer = Deframer::default();
        deframer.push(whole.slice(..HEADER_LEN + 3));

        assert_eq!(deframer.next_message().expect("well-formed"), None);
        assert!(!deframer.is_at_message_boundary());
    }

    #[test]
    fn a_compressed_message_is_refused_by_its_flag() {
        let mut deframer = Deframer::default();
        deframer.push(Bytes::from_static(&[1, 0, 0, 0, 1, b'x']));
        assert_eq!(deframer.next_message(), Err(DeframeError::Compressed));

        let mut deframer = Deframer::default();
        deframer.push(Bytes::from_static(&[7, 0, 0, 0, 1, b'x']));
        assert_eq!(
            deframer.next_message(),
            Err(DeframeError::UnknownFlag { flag: 7 })
        );
    }
}
