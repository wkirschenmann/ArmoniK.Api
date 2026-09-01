//! The raw-bytes codec the gRPC test servers route every method through.
//!
//! Its wire representation *is* the message, so one server answers any method path. Shared by
//! `#[path]` rather than through `common`, so each test binary takes this and nothing else and
//! neither reports the rest of `common` as dead.

use armonik_transport::reexports::tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use armonik_transport::reexports::tonic::Status;
use bytes::{Buf, BufMut, Bytes};

#[derive(Debug, Clone, Copy, Default)]
pub struct BytesCodec;

impl Codec for BytesCodec {
    type Encode = Bytes;
    type Decode = Bytes;
    type Encoder = Self;
    type Decoder = Self;

    fn encoder(&mut self) -> Self::Encoder {
        *self
    }

    fn decoder(&mut self) -> Self::Decoder {
        *self
    }
}

impl Encoder for BytesCodec {
    type Item = Bytes;
    type Error = Status;

    fn encode(&mut self, item: Self::Item, dst: &mut EncodeBuf<'_>) -> Result<(), Self::Error> {
        dst.reserve(item.len());
        dst.put_slice(&item);
        Ok(())
    }
}

impl Decoder for BytesCodec {
    type Item = Bytes;
    type Error = Status;

    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Self::Error> {
        let len = src.remaining();
        Ok(Some(src.copy_to_bytes(len)))
    }
}
