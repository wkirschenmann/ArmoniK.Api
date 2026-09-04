use std::collections::VecDeque;

use bytes::buf::Chain;
use bytes::{Buf, BufMut, Bytes, BytesMut};
use snafu::Snafu;

use super::error::CallError;
use super::status::{GrpcStatus, GrpcStatusCode};

const HEADER_LEN: usize = 5;

const UNCOMPRESSED: u8 = 0;

pub(crate) fn frame(payload: Bytes) -> Result<Chain<Bytes, Bytes>, CallError> {
    let len = u32::try_from(payload.len())
        .map_err(|_| CallError::MessageTooLong { len: payload.len() })?;

    let mut header = BytesMut::with_capacity(HEADER_LEN);
    header.put_u8(UNCOMPRESSED);
    header.put_u32(len);

    Ok(header.freeze().chain(payload))
}

#[derive(Debug)]
pub(crate) struct Deframer {
    chunks: VecDeque<Bytes>,
    buffered: usize,
    max_message_size: usize,
}

impl Deframer {
    pub(crate) fn new(max_message_size: usize) -> Self {
        Self {
            chunks: VecDeque::new(),
            buffered: 0,
            max_message_size,
        }
    }

    pub(crate) fn push(&mut self, chunk: Bytes) {
        if chunk.is_empty() {
            return;
        }
        self.buffered += chunk.len();
        self.chunks.push_back(chunk);
    }

    pub(crate) fn is_at_message_boundary(&self) -> bool {
        self.buffered == 0
    }

    pub(crate) fn next_message(&mut self) -> Result<Option<Bytes>, DeframeError> {
        if self.buffered < HEADER_LEN {
            return Ok(None);
        }

        let mut header = [0u8; HEADER_LEN];
        self.peek(&mut header);

        match header[0] {
            UNCOMPRESSED => {}
            1 => return CompressedSnafu.fail(),
            flag => return UnknownFlagSnafu { flag }.fail(),
        }

        let len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
        if len > self.max_message_size {
            return TooLongSnafu {
                len,
                max: self.max_message_size,
            }
            .fail();
        }
        match HEADER_LEN.checked_add(len) {
            Some(whole) if self.buffered >= whole => Ok(Some(self.take(whole).slice(HEADER_LEN..))),
            _ => Ok(None),
        }
    }

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

    fn take(&mut self, count: usize) -> Bytes {
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

#[derive(Clone, Debug, Eq, PartialEq, Snafu)]
pub(crate) enum DeframeError {
    #[snafu(display(
        "the peer sent a compressed message, though this channel advertises `identity` alone"
    ))]
    Compressed,
    #[snafu(display("a message carried the compression flag {flag}, which gRPC does not define"))]
    UnknownFlag { flag: u8 },
    #[snafu(display(
        "the peer announced a message of {len} bytes, past the {max} this channel holds"
    ))]
    TooLong { len: usize, max: usize },
}

impl DeframeError {
    pub(crate) fn status(&self) -> GrpcStatus {
        let code = match self {
            Self::TooLong { .. } => GrpcStatusCode::RESOURCE_EXHAUSTED,
            _ => GrpcStatusCode::INTERNAL,
        };
        GrpcStatus::new(code, self.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn framed(payload: &'static [u8]) -> Bytes {
        let mut chained = frame(Bytes::from_static(payload)).expect("short enough");
        chained.copy_to_bytes(chained.remaining())
    }

    fn unbounded() -> Deframer {
        Deframer::new(usize::MAX)
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
        let mut deframer = unbounded();
        deframer.push(chunk.clone());

        let message = deframer
            .next_message()
            .expect("well-formed")
            .expect("a whole message");
        assert_eq!(&message[..], b"payload");
        assert_eq!(
            message.as_ptr() as usize,
            chunk.as_ptr() as usize + HEADER_LEN
        );
        assert!(deframer.is_at_message_boundary());
    }

    #[test]
    fn a_message_split_across_chunks_is_reassembled() {
        let whole = framed(b"across the chunks");
        let mut deframer = unbounded();
        for byte in whole.iter() {
            assert_eq!(deframer.next_message().expect("well-formed"), None);
            deframer.push(Bytes::copy_from_slice(&[*byte]));
        }

        assert_eq!(
            drain(&mut deframer),
            vec![Bytes::from_static(b"across the chunks")]
        );
        assert!(deframer.is_at_message_boundary());
    }

    #[test]
    fn several_messages_in_one_chunk_come_out_in_order() {
        let mut joined = BytesMut::new();
        joined.extend_from_slice(&framed(b"one"));
        joined.extend_from_slice(&framed(b""));
        joined.extend_from_slice(&framed(b"three"));

        let mut deframer = unbounded();
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
        let mut deframer = unbounded();
        deframer.push(whole.slice(..HEADER_LEN + 3));

        assert_eq!(deframer.next_message().expect("well-formed"), None);
        assert!(!deframer.is_at_message_boundary());
    }

    #[test]
    fn a_length_that_cannot_be_reached_waits_rather_than_reaching_past_the_buffer() {
        let mut deframer = unbounded();
        deframer.push(Bytes::from_static(&[0, 0xff, 0xff, 0xff, 0xff, b'x']));

        assert_eq!(deframer.next_message(), Ok(None));
        assert!(!deframer.is_at_message_boundary());
    }

    #[test]
    fn a_message_past_the_maximum_is_refused_on_its_announced_length() {
        let mut deframer = Deframer::new(8);
        deframer.push(Bytes::from_static(&[0, 0, 0, 0, 9]));

        let refused = deframer
            .next_message()
            .expect_err("nine bytes are past a maximum of eight");
        assert_eq!(refused, DeframeError::TooLong { len: 9, max: 8 });
        assert_eq!(refused.status().code, GrpcStatusCode::RESOURCE_EXHAUSTED);

        let mut deframer = Deframer::new(8);
        deframer.push(framed(b"12345678"));
        assert_eq!(
            deframer.next_message().expect("exactly the maximum fits"),
            Some(Bytes::from_static(b"12345678"))
        );
    }

    #[test]
    fn a_compressed_message_is_refused_by_its_flag() {
        let mut deframer = unbounded();
        deframer.push(Bytes::from_static(&[1, 0, 0, 0, 1, b'x']));
        assert_eq!(deframer.next_message(), Err(DeframeError::Compressed));

        let mut deframer = unbounded();
        deframer.push(Bytes::from_static(&[7, 0, 0, 0, 1, b'x']));
        assert_eq!(
            deframer.next_message(),
            Err(DeframeError::UnknownFlag { flag: 7 })
        );
    }
}
