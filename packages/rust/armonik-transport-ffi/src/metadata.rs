//! Converting gRPC metadata across the ABI.
//!
//! gRPC metadata is an ordered multi-map of ASCII or binary values on either side of this ABI, which
//! maps onto the shared key/value encoding in [`crate::blob`] directly, duplicate keys included.
//!
//! The one wrinkle is binary values. gRPC stores a `-bin` suffixed value base64-encoded on the wire,
//! and `tonic` exposes that stored form through `AsRef<[u8]>`. This blob carries the *decoded* bytes
//! instead, which is what a caller's own gRPC library would hand it, so the conversion goes through
//! `MetadataValue::to_bytes` on the way out and `from_bytes` on the way in.

use armonik_transport::reexports::tonic::metadata::{
    Ascii, Binary, KeyAndValueRef, MetadataKey, MetadataMap, MetadataValue,
};

use crate::error::{ak_bytes, FfiError};

/// Whether a metadata key names a binary value, i.e. carries the `-bin` suffix gRPC reserves.
///
/// Case-insensitive, because header names are: a key arriving as `X-Trace-BIN` is the same key as
/// `x-trace-bin`, and testing the raw bytes would send it down the ASCII path instead, where its
/// binary value would be rejected as malformed text.
fn is_binary(key: &[u8]) -> bool {
    const SUFFIX: &[u8] = b"-bin";
    key.len() >= SUFFIX.len() && key[key.len() - SUFFIX.len()..].eq_ignore_ascii_case(SUFFIX)
}

/// Parse a metadata blob into a [`MetadataMap`].
///
/// # Safety
///
/// `data` must point to `len` valid bytes for the duration of this call.
pub(crate) unsafe fn decode(data: *const u8, len: usize) -> Result<MetadataMap, FfiError> {
    // SAFETY: forwarded from this function's own contract.
    let pairs = unsafe { crate::blob::decode(data, len) }?;

    let mut map = MetadataMap::with_capacity(pairs.len());
    for (key, value) in pairs {
        if is_binary(key) {
            let key = MetadataKey::<Binary>::from_bytes(key)
                .map_err(|_| FfiError::InvalidState("a binary metadata key was malformed"))?;
            map.append_bin(key, MetadataValue::<Binary>::from_bytes(value));
        } else {
            let key = MetadataKey::<Ascii>::from_bytes(key)
                .map_err(|_| FfiError::InvalidState("an ascii metadata key was malformed"))?;
            let value = MetadataValue::<Ascii>::try_from(value)
                .map_err(|_| FfiError::InvalidState("an ascii metadata value was malformed"))?;
            map.append(key, value);
        }
    }

    Ok(map)
}

/// Serialise a [`MetadataMap`] into a blob.
pub(crate) fn encode(map: &MetadataMap) -> Result<ak_bytes, FfiError> {
    // Binary values have to be decoded out of their base64 storage, which needs somewhere to live
    // for as long as the borrowed pairs do.
    let mut owned: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(map.len());

    for entry in map.iter() {
        match entry {
            KeyAndValueRef::Ascii(key, value) => {
                owned.push((key.as_str().as_bytes().to_vec(), value.as_ref().to_vec()));
            }
            KeyAndValueRef::Binary(key, value) => {
                // A value that fails to decode would mean `tonic` stored something it could not have
                // produced; treat it as empty rather than failing the whole call over it.
                let decoded = value
                    .to_bytes()
                    .map(|bytes| bytes.to_vec())
                    .unwrap_or_default();
                owned.push((key.as_str().as_bytes().to_vec(), decoded));
            }
        }
    }

    crate::blob::encode(
        owned
            .iter()
            .map(|(key, value)| (key.as_slice(), value.as_slice())),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_entries_round_trip_with_duplicates_preserved() {
        let mut map = MetadataMap::new();
        map.append("authorization", "Bearer token".parse().unwrap());
        map.append("x-repeated", "one".parse().unwrap());
        map.append("x-repeated", "two".parse().unwrap());

        let encoded = encode(&map).expect("encode");
        // SAFETY: just produced above; freed at the end of this test.
        let decoded = unsafe { decode(encoded.ptr, encoded.len) }.expect("decode");

        assert_eq!(
            decoded.get("authorization").unwrap().to_str().unwrap(),
            "Bearer token"
        );
        assert_eq!(decoded.get_all("x-repeated").iter().count(), 2);

        unsafe { crate::error::ak_bytes_free(encoded) };
    }

    #[test]
    fn binary_entries_round_trip_raw_bytes() {
        let mut map = MetadataMap::new();
        let payload: &[u8] = &[0, 1, 2, 0xff, 0xfe, 0x00];
        map.append_bin("x-trace-bin", MetadataValue::from_bytes(payload));

        let encoded = encode(&map).expect("encode");
        // SAFETY: just produced above; freed at the end of this test.
        let decoded = unsafe { decode(encoded.ptr, encoded.len) }.expect("decode");

        // `.as_ref()` on a binary value gives its base64 wire storage, not the original bytes;
        // `.to_bytes()` is what decodes it back. Using the wrong one is exactly the mistake `encode`
        // has to avoid, so pin it down with the accessor `decode` is meant to pair with.
        assert_eq!(
            decoded.get_bin("x-trace-bin").unwrap().to_bytes().unwrap(),
            payload
        );

        unsafe { crate::error::ak_bytes_free(encoded) };
    }

    #[test]
    fn an_empty_map_round_trips_to_an_empty_map() {
        let encoded = encode(&MetadataMap::new()).expect("encode");
        // SAFETY: just produced above; freed below.
        let decoded = unsafe { decode(encoded.ptr, encoded.len) }.expect("decode");
        assert_eq!(decoded.len(), 0);
        unsafe { crate::error::ak_bytes_free(encoded) };
    }

    #[test]
    fn a_null_blob_decodes_to_an_empty_map() {
        // SAFETY: the null/zero case is explicitly allowed.
        let map = unsafe { decode(std::ptr::null(), 0) }.expect("decode");
        assert_eq!(map.len(), 0);
    }

    /// Encode a single-entry blob by hand, bypassing [`encode`] so a key it would never produce can
    /// still be fed to [`decode`].
    fn one_entry_blob(key: &[u8], value: &[u8]) -> Vec<u8> {
        let mut blob = 1u32.to_ne_bytes().to_vec();
        blob.extend_from_slice(&(key.len() as u32).to_ne_bytes());
        blob.extend_from_slice(key);
        blob.extend_from_slice(&(value.len() as u32).to_ne_bytes());
        blob.extend_from_slice(value);
        blob
    }

    #[test]
    fn a_key_grpc_would_reject_is_reported_rather_than_appended() {
        // A space is illegal in an HTTP/2 header name. The blob format is happy to carry it, so this
        // rejection has to come from here rather than from the encoding.
        let blob = one_entry_blob(b"not valid", b"v");
        // SAFETY: `blob` is a live slice for its own length.
        assert!(unsafe { decode(blob.as_ptr(), blob.len()) }.is_err());
    }

    #[test]
    fn an_uppercase_key_is_lowercased_rather_than_rejected() {
        // HTTP header names are case-insensitive and normalised to lower case, which is what both
        // `tonic` and every other gRPC library do. Worth pinning down, because the obvious guess is that
        // an upper-case name is simply invalid — it is not, it silently changes case, and a caller
        // that looked the key back up by its original spelling would find nothing.
        let blob = one_entry_blob(b"X-Mixed-Case", b"v");
        // SAFETY: `blob` is a live slice for its own length.
        let decoded = unsafe { decode(blob.as_ptr(), blob.len()) }.expect("decode");

        assert_eq!(decoded.get("x-mixed-case").unwrap().to_str().unwrap(), "v");
        assert_eq!(decoded.len(), 1);
    }

    #[test]
    fn a_binary_key_is_recognised_by_its_suffix_whatever_its_case() {
        // The `-bin` test has to survive the lower-casing above, or a `-BIN` key would be stored as
        // ASCII and its value base64-mangled.
        let payload: &[u8] = &[0, 0xff];
        let blob = one_entry_blob(b"X-Trace-BIN", payload);
        // SAFETY: `blob` is a live slice for its own length.
        let decoded = unsafe { decode(blob.as_ptr(), blob.len()) }.expect("decode");

        assert_eq!(
            decoded.get_bin("x-trace-bin").unwrap().to_bytes().unwrap(),
            payload
        );
    }

    #[test]
    fn a_truncated_blob_is_rejected() {
        let truncated = 1u32.to_ne_bytes();
        // SAFETY: a live slice for its own length.
        assert!(unsafe { decode(truncated.as_ptr(), truncated.len()) }.is_err());
    }
}
