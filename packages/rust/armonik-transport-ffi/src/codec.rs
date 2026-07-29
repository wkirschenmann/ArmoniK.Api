//! The raw-bytes gRPC codec that lets one FFI contract route every method.
//!
//! Every other language binding in this repository has protoc generate a typed `Encoder`/`Decoder`
//! pair per message. This crate deliberately has none of that: [`BytesCodec`] moves the encoded
//! protobuf bytes verbatim, and the .NET side supplies its own `Marshaller<T>` (backed by the
//! `Google.Protobuf`-generated types it already has) to interpret them. That is what lets this
//! crate route any method path without a line of code per RPC.

use armonik_transport::reexports::tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use armonik_transport::reexports::tonic::Status;
use bytes::{Buf, BufMut, Bytes};

/// A [`Codec`] whose wire representation *is* the message: no framing beyond what gRPC itself
/// already adds.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct BytesCodec;

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

// No unit tests here: `tonic::codec::{EncodeBuf, DecodeBuf}` only expose a `pub(crate)`
// constructor, so this codec cannot be driven directly from outside `tonic`. It is exercised
// end-to-end instead, by every test in `tests/calls.rs` that sends and receives a message — those
// prove the round trip (including the empty-message case, since several ArmoniK RPCs have request
// or response types with no fields set) through the real `tonic::client::Grpc` pipeline this codec
// actually runs under.
