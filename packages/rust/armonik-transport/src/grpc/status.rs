//! The terminal of a call, and how a response head or a set of trailers is read as one.
//!
//! The only place the wire's spelling of a status is understood.

use http::header::{HeaderMap, CONTENT_TYPE};
use http::StatusCode;

use tonic::Code;

use super::metadata::Metadata;

/// The two trailers a status travels in. Here rather than with the other header names: this
/// module is the only thing that reads them, and a reader looking for where `grpc-status` is
/// understood should find the spelling in the same file as the understanding.
pub(crate) const GRPC_STATUS: &str = "grpc-status";
pub(crate) const GRPC_MESSAGE: &str = "grpc-message";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
/// The gRPC status codes, as they travel in `grpc-status`.
///
/// A wrapper over [`tonic::Code`] rather than a second enumeration of the same closed space.
/// `tonic` is already a hard dependency of this crate and defines these seventeen codes and their
/// numbering; writing them out again would be two things to keep in step for one wire vocabulary,
/// and nothing would notice if they drifted.
///
/// What the wrapper adds is what this engine needs and `tonic` does not offer: the spelling gRPC
/// puts on the wire - `CANCELLED`, not "The operation was cancelled" - and the mapping from an
/// HTTP status. Shaped like [`http::StatusCode`], which this crate already uses: a newtype with a
/// named constant per value.
pub struct GrpcStatusCode(Code);

impl GrpcStatusCode {
    pub const OK: Self = Self(Code::Ok);
    pub const CANCELLED: Self = Self(Code::Cancelled);
    pub const UNKNOWN: Self = Self(Code::Unknown);
    pub const INVALID_ARGUMENT: Self = Self(Code::InvalidArgument);
    pub const DEADLINE_EXCEEDED: Self = Self(Code::DeadlineExceeded);
    pub const NOT_FOUND: Self = Self(Code::NotFound);
    pub const ALREADY_EXISTS: Self = Self(Code::AlreadyExists);
    pub const PERMISSION_DENIED: Self = Self(Code::PermissionDenied);
    pub const RESOURCE_EXHAUSTED: Self = Self(Code::ResourceExhausted);
    pub const FAILED_PRECONDITION: Self = Self(Code::FailedPrecondition);
    pub const ABORTED: Self = Self(Code::Aborted);
    pub const OUT_OF_RANGE: Self = Self(Code::OutOfRange);
    pub const UNIMPLEMENTED: Self = Self(Code::Unimplemented);
    pub const INTERNAL: Self = Self(Code::Internal);
    pub const UNAVAILABLE: Self = Self(Code::Unavailable);
    pub const DATA_LOSS: Self = Self(Code::DataLoss);
    pub const UNAUTHENTICATED: Self = Self(Code::Unauthenticated);

    /// The code as `tonic` names it, for a caller already speaking that vocabulary.
    pub const fn code(self) -> Code {
        self.0
    }

    /// The number this code travels as, which is also what the C ABI above carries.
    pub const fn as_i32(self) -> i32 {
        self.0 as i32
    }

    /// A value outside the range is `UNKNOWN`: the space is closed, so a code nobody defined
    /// carries no more than "the call failed and we cannot say how".
    pub const fn from_wire(code: i32) -> Self {
        Self(Code::from_i32(code))
    }

    /// The code a response that never became gRPC maps to. The table is the one in the gRPC
    /// HTTP/2 specification; anything it does not name is `UNKNOWN`.
    ///
    /// Private: a caller of this engine is answered in gRPC codes, which is what
    /// [`of_response_head`] is for.
    fn from_http_status(status: u16) -> Self {
        match status {
            400 => Self::INTERNAL,
            401 => Self::UNAUTHENTICATED,
            403 => Self::PERMISSION_DENIED,
            404 => Self::UNIMPLEMENTED,
            429 | 502 | 503 | 504 => Self::UNAVAILABLE,
            _ => Self::UNKNOWN,
        }
    }
}

impl From<Code> for GrpcStatusCode {
    fn from(code: Code) -> Self {
        Self(code)
    }
}

impl From<GrpcStatusCode> for Code {
    fn from(code: GrpcStatusCode) -> Self {
        code.0
    }
}

impl std::fmt::Display for GrpcStatusCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self.0 {
            Code::Ok => "OK",
            Code::Cancelled => "CANCELLED",
            Code::Unknown => "UNKNOWN",
            Code::InvalidArgument => "INVALID_ARGUMENT",
            Code::DeadlineExceeded => "DEADLINE_EXCEEDED",
            Code::NotFound => "NOT_FOUND",
            Code::AlreadyExists => "ALREADY_EXISTS",
            Code::PermissionDenied => "PERMISSION_DENIED",
            Code::ResourceExhausted => "RESOURCE_EXHAUSTED",
            Code::FailedPrecondition => "FAILED_PRECONDITION",
            Code::Aborted => "ABORTED",
            Code::OutOfRange => "OUT_OF_RANGE",
            Code::Unimplemented => "UNIMPLEMENTED",
            Code::Internal => "INTERNAL",
            Code::Unavailable => "UNAVAILABLE",
            Code::DataLoss => "DATA_LOSS",
            Code::Unauthenticated => "UNAUTHENTICATED",
        };
        f.write_str(name)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// How a call ended.
pub struct GrpcStatus {
    /// The code the server sent, or the one the failure maps to.
    pub code: GrpcStatusCode,
    /// The `grpc-message`, percent-decoded. Empty when the call succeeded.
    pub message: String,
    /// The trailers, minus `grpc-status` and `grpc-message`, which are this status.
    pub trailing_metadata: Metadata,
}

impl GrpcStatus {
    pub fn new(code: GrpcStatusCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            trailing_metadata: Metadata::new(),
        }
    }

    /// The terminal of a call nobody is waiting for any more, wherever that was decided.
    ///
    /// Public because the boundary above this crate ends an abandoned call with it too, and a
    /// second spelling of the same terminal would be a second thing to keep in step.
    pub fn cancelled() -> Self {
        Self::new(GrpcStatusCode::CANCELLED, "the call was cancelled")
    }

    pub(crate) fn unreachable(error: impl std::fmt::Display) -> Self {
        Self::new(GrpcStatusCode::UNAVAILABLE, error.to_string())
    }

    pub(crate) fn request_lost(error: impl std::fmt::Display) -> Self {
        Self::new(
            GrpcStatusCode::UNAVAILABLE,
            format!("the request did not reach the peer: {error}"),
        )
    }

    pub(crate) fn stream_broke(error: impl std::fmt::Display) -> Self {
        Self::new(
            GrpcStatusCode::UNAVAILABLE,
            format!("the response stream broke: {error}"),
        )
    }

    pub(crate) fn not_grpc() -> Self {
        Self::new(
            GrpcStatusCode::INTERNAL,
            "the peer answered HTTP 200 without a gRPC content type",
        )
    }

    pub(crate) fn no_status() -> Self {
        Self::new(
            GrpcStatusCode::INTERNAL,
            "the peer ended the stream without a grpc-status",
        )
    }

    pub(crate) fn no_trailing_status() -> Self {
        Self::new(
            GrpcStatusCode::INTERNAL,
            "the peer's trailers carry no grpc-status",
        )
    }

    pub(crate) fn ended_mid_message() -> Self {
        Self::new(
            GrpcStatusCode::INTERNAL,
            "the peer ended the stream in the middle of a message",
        )
    }
}

impl std::fmt::Display for GrpcStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.message.is_empty() {
            write!(f, "{}", self.code)
        } else {
            write!(f, "{}: {}", self.code, self.message)
        }
    }
}

/// What a response head means: the metadata to hand over, or the terminal to end on.
///
/// The three readings are ordered, and the order is the substance. A peer that states a status in
/// the head has said how the call ended, and that answer stands whatever the HTTP status is: a
/// Trailers-Only response is this case, and so is a gRPC failure served behind an HTTP error.
pub(crate) fn of_response_head(
    status: StatusCode,
    headers: &HeaderMap,
) -> Result<Metadata, GrpcStatus> {
    if let Some(stated) = stated_status(headers) {
        return Err(stated);
    }
    if status != StatusCode::OK {
        return Err(http_status(status, headers));
    }
    if !speaks_grpc(headers) {
        return Err(GrpcStatus::not_grpc());
    }
    Ok(Metadata::from_headers(headers))
}

/// The terminal a set of headers or trailers states, if it states one.
pub(crate) fn stated_status(headers: &HeaderMap) -> Option<GrpcStatus> {
    let raw = headers.get(GRPC_STATUS)?;

    let code = raw
        .to_str()
        .ok()
        .and_then(|text| text.trim().parse::<i32>().ok());

    let Some(code) = code else {
        return Some(GrpcStatus::new(
            GrpcStatusCode::INTERNAL,
            "the peer's grpc-status is not a number",
        ));
    };

    Some(GrpcStatus {
        code: GrpcStatusCode::from_wire(code),
        message: headers
            .get(GRPC_MESSAGE)
            .map(|value| decode_message(value.as_bytes()))
            .unwrap_or_default(),
        trailing_metadata: Metadata::from_headers(headers),
    })
}

fn http_status(status: StatusCode, headers: &HeaderMap) -> GrpcStatus {
    GrpcStatus {
        code: GrpcStatusCode::from_http_status(status.as_u16()),
        message: format!(
            "the peer answered HTTP {} rather than gRPC",
            status.as_u16()
        ),
        trailing_metadata: Metadata::from_headers(headers),
    }
}

/// Whether the content type says the body is gRPC. The subtype after `+` names the message
/// encoding, which is the caller's business rather than this engine's; media types are
/// case-insensitive, so the comparison is too.
fn speaks_grpc(headers: &HeaderMap) -> bool {
    const GRPC: &str = "application/grpc";

    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            let value = value.trim();
            value.len() >= GRPC.len()
                && value[..GRPC.len()].eq_ignore_ascii_case(GRPC)
                && value[GRPC.len()..]
                    .chars()
                    .next()
                    .is_none_or(|next| next == '+' || next == ';')
        })
        .unwrap_or(false)
}

/// Percent-decoding for `grpc-message`.
///
/// An invalid escape is kept verbatim rather than rejected: this is a human-readable reason for
/// a failure that has already happened, and refusing to read it would replace the server's
/// account of the failure with an account of the encoding.
fn decode_message(raw: &[u8]) -> String {
    // Nothing to decode is the ordinary case, and it borrows rather than rebuilding.
    if !raw.contains(&b'%') {
        return String::from_utf8_lossy(raw).into_owned();
    }

    let mut out = Vec::with_capacity(raw.len());
    let mut index = 0;
    while index < raw.len() {
        match raw[index] {
            b'%' if index + 2 < raw.len() => match (hex(raw[index + 1]), hex(raw[index + 2])) {
                (Some(high), Some(low)) => {
                    out.push(high << 4 | low);
                    index += 3;
                }
                _ => {
                    out.push(raw[index]);
                    index += 1;
                }
            },
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    // The decoded bytes are text in every case but a peer that escaped invalid UTF-8, so the
    // buffer is taken as it stands rather than copied into a second one.
    String::from_utf8(out)
        .unwrap_or_else(|invalid| String::from_utf8_lossy(invalid.as_bytes()).into_owned())
}

fn hex(byte: u8) -> Option<u8> {
    char::from(byte).to_digit(16).map(|digit| digit as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::header::HeaderValue;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (key, value) in pairs {
            map.append(*key, HeaderValue::from_static(value));
        }
        map
    }

    #[test]
    fn a_code_outside_the_defined_range_reads_as_unknown() {
        assert_eq!(GrpcStatusCode::from_wire(14), GrpcStatusCode::UNAVAILABLE);
        assert_eq!(GrpcStatusCode::from_wire(2), GrpcStatusCode::UNKNOWN);
        assert_eq!(GrpcStatusCode::from_wire(99), GrpcStatusCode::UNKNOWN);
        assert_eq!(GrpcStatusCode::from_wire(-1), GrpcStatusCode::UNKNOWN);
    }

    #[test]
    fn a_percent_escape_is_decoded_and_a_broken_one_is_kept() {
        assert_eq!(decode_message(b"plain"), "plain");
        assert_eq!(decode_message(b"a%20b"), "a b");
        // A truncated or non-hex escape is what the server sent; it is not ours to drop.
        assert_eq!(decode_message(b"100%"), "100%");
        assert_eq!(decode_message(b"a%zzb"), "a%zzb");
    }

    #[test]
    fn a_decoded_message_that_is_not_utf8_is_replaced_rather_than_refused() {
        assert_eq!(decode_message(b"%ff"), "\u{fffd}");
    }

    #[test]
    fn a_status_carries_its_code_its_reason_and_the_rest_of_the_trailers() {
        let status = stated_status(&headers(&[
            ("grpc-status", "9"),
            ("grpc-message", "not%20now"),
            ("x-trailer", "kept"),
        ]))
        .expect("these trailers state a status");

        assert_eq!(status.code, GrpcStatusCode::FAILED_PRECONDITION);
        assert_eq!(status.message, "not now");
        assert_eq!(status.trailing_metadata.len(), 1);
    }

    #[test]
    fn headers_without_a_status_state_none() {
        assert_eq!(stated_status(&headers(&[("grpc-message", "orphan")])), None);
        assert_eq!(stated_status(&HeaderMap::new()), None);
    }

    #[test]
    fn a_status_that_is_not_a_number_is_an_internal_failure() {
        assert_eq!(
            stated_status(&headers(&[("grpc-status", "not a number")]))
                .expect("the header is there")
                .code,
            GrpcStatusCode::INTERNAL
        );
    }

    #[test]
    fn a_status_without_a_message_is_still_a_status() {
        let status = stated_status(&headers(&[("grpc-status", "0")])).expect("a status");
        assert_eq!(status.code, GrpcStatusCode::OK);
        assert!(status.message.is_empty());
    }

    #[test]
    fn the_content_type_has_to_say_grpc() {
        for value in [
            "application/grpc",
            "application/grpc+proto",
            "application/grpc; charset=utf-8",
            "Application/gRPC",
        ] {
            let mut map = HeaderMap::new();
            map.insert("content-type", HeaderValue::from_str(value).expect("valid"));
            assert!(speaks_grpc(&map), "{value}");
        }

        assert!(!speaks_grpc(&headers(&[("content-type", "text/html")])));
        assert!(!speaks_grpc(&headers(&[(
            "content-type",
            "application/grpcweb"
        )])));
        assert!(!speaks_grpc(&HeaderMap::new()));
    }

    #[test]
    fn an_http_failure_maps_to_the_code_grpc_gives_it() {
        assert_eq!(
            http_status(StatusCode::NOT_FOUND, &HeaderMap::new()).code,
            GrpcStatusCode::UNIMPLEMENTED
        );
        assert_eq!(
            http_status(StatusCode::SERVICE_UNAVAILABLE, &HeaderMap::new()).code,
            GrpcStatusCode::UNAVAILABLE
        );
        assert_eq!(
            http_status(StatusCode::IM_A_TEAPOT, &HeaderMap::new()).code,
            GrpcStatusCode::UNKNOWN
        );
    }
}
