//! Call metadata: an ordered multi-map of ASCII or binary values.
//!
//! A key ending in `-bin` carries bytes, and this type holds them decoded. gRPC stores such a
//! value base64-encoded on the wire, so a caller never sees the wire form.

use super::status;
use base64::alphabet;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use base64::Engine;
use bytes::Bytes;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use snafu::Snafu;

/// Padding is optional on the wire, so both forms decode.
///
/// Configured here rather than taken from `tonic`, whose `metadata` module holds the identical
/// pair: its engines are private, and reaching them through `MetadataValue<Binary>` would mean
/// holding the base64 text instead of the bytes. This type holds a `-bin` value decoded, which is
/// what lets the C ABI above hand a host raw bytes with no fallible decode at the boundary. The
/// duplication is deliberate, and it is why `base64` is a dependency.
const BINARY_IN: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);
const BINARY_OUT: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_encode_padding(false)
        .with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

pub const BINARY_SUFFIX: &str = "-bin";

#[derive(Clone, Debug, Eq, PartialEq)]
/// One metadata value, in the form its key implies.
pub enum MetadataValue {
    Ascii(String),
    Binary(Bytes),
}

impl MetadataValue {
    /// The value as bytes, whichever form it is in. What a consumer that carries values rather
    /// than reads them wants, and the only place the two forms have to be told apart for it.
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Ascii(text) => text.as_bytes(),
            Self::Binary(bytes) => bytes,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
/// The metadata of a request, a response head, or a set of trailers.
pub struct Metadata {
    /// Keys as `HeaderName` and not `String`: the name is validated and lowercased once, on the
    /// way in, and a header map then takes it by clone - a refcount bump for a standard name.
    /// Holding the text instead meant re-parsing and re-allocating every key on every call that
    /// carried it, which is exactly what an auth token built once and sent on every call does.
    entries: Vec<(HeaderName, MetadataValue)>,
}

impl Metadata {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Adds an entry, keeping any entry already under that key.
    ///
    /// The key is lowercased: header names are case-insensitive, and holding two spellings of
    /// one key would make `get` depend on which the caller used.
    pub fn append(&mut self, key: &str, value: MetadataValue) -> Result<(), MetadataError> {
        let key = validate_key(key)?;
        validate_value(key.as_str(), &value)?;
        self.entries.push((key, value));
        Ok(())
    }

    pub fn append_ascii(
        &mut self,
        key: &str,
        value: impl Into<String>,
    ) -> Result<(), MetadataError> {
        self.append(key, MetadataValue::Ascii(value.into()))
    }

    /// Adds a binary entry. The key must carry the `-bin` suffix.
    pub fn append_binary(
        &mut self,
        key: &str,
        value: impl Into<Bytes>,
    ) -> Result<(), MetadataError> {
        self.append(key, MetadataValue::Binary(value.into()))
    }

    pub fn get(&self, key: &str) -> Option<&MetadataValue> {
        self.get_all(key).next()
    }

    /// The key is borrowed for the life of the iterator rather than copied: `append` lowercased
    /// what is stored, so a case-insensitive comparison is the whole of the lookup and there is
    /// nothing to own.
    pub fn get_all<'a, 'k>(
        &'a self,
        key: &'k str,
    ) -> impl Iterator<Item = &'a MetadataValue> + use<'a, 'k> {
        self.entries
            .iter()
            .filter(move |(name, _)| name.as_str().eq_ignore_ascii_case(key))
            .map(|(_, value)| value)
    }

    /// `Clone` so a consumer that has to size a buffer before filling it can walk the entries
    /// twice without asking for a second borrow.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&str, &MetadataValue)> + Clone {
        self.entries
            .iter()
            .map(|(key, value)| (key.as_str(), value))
    }

    /// Reads what a header map carries, skipping the two headers that are the status rather
    /// than metadata.
    ///
    /// A header this type cannot represent is dropped rather than failing the call: the response
    /// has already arrived, and refusing it here would turn a peer's malformed header into a
    /// lost result. What is kept is exactly what could be sent again, so a head read here can be
    /// forwarded onto a request without a value in it turning out to be unsendable.
    pub(crate) fn from_headers(headers: &HeaderMap) -> Self {
        let mut entries = Vec::with_capacity(headers.len());
        for (name, raw) in headers {
            let key = name.as_str();
            if key == status::GRPC_STATUS || key == status::GRPC_MESSAGE {
                continue;
            }
            let value = if key.ends_with(BINARY_SUFFIX) {
                match BINARY_IN.decode(raw.as_bytes()) {
                    Ok(bytes) => MetadataValue::Binary(Bytes::from(bytes)),
                    Err(_) => continue,
                }
            } else {
                match std::str::from_utf8(raw.as_bytes()) {
                    Ok(text) => MetadataValue::Ascii(text.to_owned()),
                    Err(_) => continue,
                }
            };
            if validate_value(key, &value).is_err() {
                continue;
            }
            // The name is cloned, not re-parsed: it came out of a header map already validated,
            // and for a standard name the clone is a refcount bump.
            entries.push((name.clone(), value));
        }
        Self { entries }
    }

    /// Room for these entries on top of what `headers` already holds.
    ///
    /// `HeaderMap` panics rather than growing past its ceiling, and this count is the caller's.
    pub(crate) fn reserve_in(&self, headers: &mut HeaderMap) -> Result<(), MetadataError> {
        headers
            .try_reserve(self.entries.len())
            .map_err(|_| MetadataError::TooMany {
                entries: self.entries.len(),
            })
    }

    /// Writes these entries into the headers of a request.
    ///
    /// A key the channel owns is skipped rather than refused. `append` turns one away, so the
    /// only way one gets here is [`Self::from_headers`], and forwarding a response head onto a
    /// new request is a reasonable thing to do; failing it over the `content-type` this engine
    /// put there itself would not be.
    pub(crate) fn write_into(&self, headers: &mut HeaderMap) -> Result<(), MetadataError> {
        for (key, value) in &self.entries {
            if is_reserved(key.as_str()) {
                continue;
            }

            // The key needs no conversion: it was a `HeaderName` before it was stored, and a
            // clone of one is a refcount bump. Both constructors validate the value too, so the
            // conversion below cannot fail on what is stored; it guards against a third way of
            // building one of these. What can fail is the map filling up.
            let encoded = match value {
                MetadataValue::Ascii(text) => {
                    HeaderValue::from_str(text).map_err(|_| MetadataError::InvalidValue {
                        key: key.as_str().to_owned(),
                    })?
                }
                MetadataValue::Binary(bytes) => HeaderValue::from_maybe_shared(Bytes::from(
                    BINARY_OUT.encode(bytes),
                ))
                .map_err(|_| MetadataError::InvalidValue {
                    key: key.as_str().to_owned(),
                })?,
            };
            headers
                .try_append(key.clone(), encoded)
                .map_err(|_| MetadataError::TooMany {
                    entries: self.entries.len(),
                })?;
        }
        Ok(())
    }
}

/// Whether gRPC or HTTP owns this key, so a caller may not set it.
///
/// `content-type` and `te` say what protocol is being spoken and `user-agent` comes from the
/// channel configuration; a second value for any of them would go out alongside ours rather than
/// replace it, since header fields repeat.
fn is_reserved(key: &str) -> bool {
    key.starts_with(':')
        || key.starts_with("grpc-")
        || matches!(key, "content-type" | "te" | "user-agent")
}

/// The key a caller may store under, or why it may not.
///
/// The reserved check runs first because a pseudo-header is not a header name at all: asking
/// `HeaderName` about `:path` would report its spelling as the fault, where what is wrong with
/// it is whose it is.
fn validate_key(key: &str) -> Result<HeaderName, MetadataError> {
    let lowered = key.to_ascii_lowercase();
    if is_reserved(&lowered) {
        return Err(MetadataError::ReservedKey { key: lowered });
    }
    HeaderName::from_bytes(lowered.as_bytes())
        .map_err(|_| MetadataError::InvalidKey { key: lowered })
}

fn validate_value(key: &str, value: &MetadataValue) -> Result<(), MetadataError> {
    match (key.ends_with(BINARY_SUFFIX), value) {
        (true, MetadataValue::Binary(_)) => Ok(()),
        (false, MetadataValue::Ascii(text)) => {
            if text.bytes().all(|byte| (0x20..=0x7e).contains(&byte)) {
                Ok(())
            } else {
                Err(MetadataError::InvalidValue {
                    key: key.to_owned(),
                })
            }
        }
        _ => Err(MetadataError::BinaryMismatch {
            key: key.to_owned(),
        }),
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Snafu)]
#[non_exhaustive]
/// Why an entry could not be stored, or written out to a header map.
pub enum MetadataError {
    #[snafu(display("`{key}` is not a valid metadata key"))]
    InvalidKey { key: String },
    #[snafu(display("`{key}` is reserved; the channel sets it, not the caller"))]
    ReservedKey { key: String },
    #[snafu(display(
        "the value under `{key}` is not printable ASCII, which a key without the `-bin` suffix \
         requires"
    ))]
    InvalidValue { key: String },
    #[snafu(display(
        "`{key}` and its value disagree on being binary; the `-bin` suffix is what decides it"
    ))]
    BinaryMismatch { key: String },
    #[snafu(display("{entries} entries is more than a header map will hold"))]
    TooMany { entries: usize },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_binary_value_survives_the_round_trip_through_headers() {
        let mut metadata = Metadata::new();
        metadata
            .append_binary("trace-bin", Bytes::from_static(&[0, 1, 2, 0xff]))
            .expect("a binary value under a binary key");

        let mut headers = HeaderMap::new();
        metadata.write_into(&mut headers).expect("valid metadata");
        // Base64 without padding is what goes on the wire, not the raw bytes.
        assert_eq!(headers["trace-bin"], "AAEC/w");

        assert_eq!(Metadata::from_headers(&headers), metadata);
    }

    #[test]
    fn a_padded_binary_value_is_accepted_on_the_way_in() {
        let mut headers = HeaderMap::new();
        headers.insert("trace-bin", HeaderValue::from_static("AAEC/w=="));

        assert_eq!(
            Metadata::from_headers(&headers).get("trace-bin"),
            Some(&MetadataValue::Binary(Bytes::from_static(&[0, 1, 2, 0xff])))
        );
    }

    #[test]
    fn repeated_keys_keep_their_order() {
        let mut metadata = Metadata::new();
        metadata.append_ascii("k", "first").expect("valid");
        metadata.append_ascii("K", "second").expect("valid");

        let values: Vec<_> = metadata.get_all("k").cloned().collect();
        assert_eq!(
            values,
            vec![
                MetadataValue::Ascii("first".to_owned()),
                MetadataValue::Ascii("second".to_owned()),
            ]
        );

        let mut headers = HeaderMap::new();
        metadata.write_into(&mut headers).expect("valid metadata");
        let round_tripped: Vec<_> = headers.get_all("k").iter().collect();
        assert_eq!(round_tripped, vec!["first", "second"]);
    }

    #[test]
    fn a_key_the_channel_owns_is_refused() {
        let mut metadata = Metadata::new();
        for key in [":path", "grpc-timeout", "content-type", "te", "user-agent"] {
            assert_eq!(
                metadata.append_ascii(key, "x"),
                Err(MetadataError::ReservedKey {
                    key: key.to_owned()
                }),
                "{key}"
            );
        }
    }

    #[test]
    fn a_value_and_its_key_have_to_agree_on_being_binary() {
        let mut metadata = Metadata::new();
        assert_eq!(
            metadata.append_ascii("trace-bin", "text"),
            Err(MetadataError::BinaryMismatch {
                key: "trace-bin".to_owned()
            })
        );
        assert_eq!(
            metadata.append_binary("trace", Bytes::from_static(b"bytes")),
            Err(MetadataError::BinaryMismatch {
                key: "trace".to_owned()
            })
        );
    }

    #[test]
    fn an_ascii_value_outside_the_printable_range_is_refused() {
        let mut metadata = Metadata::new();
        assert!(metadata.append_ascii("k", "line\r\nbreak").is_err());
        assert!(metadata.append_ascii("k", "tab\there").is_err());
        assert!(metadata.append_ascii("k", "space and ~tilde").is_ok());
    }

    #[test]
    fn more_entries_than_a_header_map_holds_is_an_error_and_not_a_panic() {
        // `HeaderMap` panics rather than growing past its ceiling, and this count is a
        // caller's to choose.
        let mut metadata = Metadata::new();
        for index in 0..40_000 {
            metadata
                .append_ascii(&format!("x-{index}"), "v")
                .expect("a plain entry");
        }

        let mut headers = HeaderMap::new();
        assert!(matches!(
            metadata.reserve_in(&mut headers),
            Err(MetadataError::TooMany { .. })
        ));
    }

    #[test]
    fn an_empty_key_is_not_a_key() {
        let mut metadata = Metadata::new();
        assert_eq!(
            metadata.append_ascii("", "x"),
            Err(MetadataError::InvalidKey { key: String::new() })
        );
    }

    #[test]
    fn a_response_head_can_be_forwarded_onto_a_request_without_its_reserved_keys() {
        let mut received = HeaderMap::new();
        received.insert("content-type", HeaderValue::from_static("application/grpc"));
        received.insert("grpc-encoding", HeaderValue::from_static("identity"));
        received.insert("x-trace", HeaderValue::from_static("kept"));

        let mut request = HeaderMap::new();
        Metadata::from_headers(&received)
            .write_into(&mut request)
            .expect("forwarding a response head is not an error");

        assert_eq!(request.len(), 1);
        assert_eq!(request["x-trace"], "kept");
    }

    #[test]
    fn the_status_headers_are_the_status_and_not_metadata() {
        let mut headers = HeaderMap::new();
        headers.insert("grpc-status", HeaderValue::from_static("5"));
        headers.insert("grpc-message", HeaderValue::from_static("gone"));
        headers.insert("x-other", HeaderValue::from_static("kept"));

        let metadata = Metadata::from_headers(&headers);
        assert_eq!(metadata.len(), 1);
        assert_eq!(
            metadata.get("x-other"),
            Some(&MetadataValue::Ascii("kept".to_owned()))
        );
    }

    #[test]
    fn a_header_this_type_cannot_represent_is_dropped_rather_than_failing_the_response() {
        let mut headers = HeaderMap::new();
        headers.insert("broken-bin", HeaderValue::from_static("not base64!"));
        // `http` allows obs-text and HTAB in a header value; gRPC ASCII metadata does not,
        // so keeping either would build metadata that cannot be written back out.
        headers.insert(
            "x-accented",
            HeaderValue::from_bytes(&[b'c', b'a', b'f', 0xe9]).expect("http allows this"),
        );
        headers.insert(
            "x-tabbed",
            HeaderValue::from_bytes(b"one	two").expect("http allows this"),
        );
        headers.insert("x-other", HeaderValue::from_static("kept"));

        let metadata = Metadata::from_headers(&headers);
        assert_eq!(metadata.len(), 1);
        assert!(metadata.get("broken-bin").is_none());
        assert!(metadata.get("x-accented").is_none());
        assert!(metadata.get("x-tabbed").is_none());

        let mut request = HeaderMap::new();
        metadata
            .write_into(&mut request)
            .expect("what came off a response can go onto a request");
    }
}
