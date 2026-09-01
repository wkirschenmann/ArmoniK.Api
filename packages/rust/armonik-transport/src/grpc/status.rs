//! The terminal of a call: a gRPC status code, a message, and the trailing metadata.

use super::metadata::Metadata;

/// The gRPC status codes, as they travel in `grpc-status`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(i32)]
pub enum GrpcStatusCode {
    Ok = 0,
    Cancelled = 1,
    Unknown = 2,
    InvalidArgument = 3,
    DeadlineExceeded = 4,
    NotFound = 5,
    AlreadyExists = 6,
    PermissionDenied = 7,
    ResourceExhausted = 8,
    FailedPrecondition = 9,
    Aborted = 10,
    OutOfRange = 11,
    Unimplemented = 12,
    Internal = 13,
    Unavailable = 14,
    DataLoss = 15,
    Unauthenticated = 16,
}

impl GrpcStatusCode {
    /// The code a `grpc-status` value names.
    ///
    /// A value outside the range is `Unknown`: the space is closed, so a code nobody defined
    /// carries no more information than "the call failed and we cannot say how".
    pub fn from_wire(code: i32) -> Self {
        match code {
            0 => Self::Ok,
            1 => Self::Cancelled,
            3 => Self::InvalidArgument,
            4 => Self::DeadlineExceeded,
            5 => Self::NotFound,
            6 => Self::AlreadyExists,
            7 => Self::PermissionDenied,
            8 => Self::ResourceExhausted,
            9 => Self::FailedPrecondition,
            10 => Self::Aborted,
            11 => Self::OutOfRange,
            12 => Self::Unimplemented,
            13 => Self::Internal,
            14 => Self::Unavailable,
            15 => Self::DataLoss,
            16 => Self::Unauthenticated,
            _ => Self::Unknown,
        }
    }

    /// The code an HTTP response status maps to, for a response that never became gRPC.
    ///
    /// The table is the one in the gRPC HTTP/2 specification; anything it does not name is
    /// `Unknown`.
    pub fn from_http_status(status: u16) -> Self {
        match status {
            400 | 431 => Self::Internal,
            401 => Self::Unauthenticated,
            403 => Self::PermissionDenied,
            404 => Self::Unimplemented,
            429 | 502 | 503 | 504 => Self::Unavailable,
            _ => Self::Unknown,
        }
    }
}

impl std::fmt::Display for GrpcStatusCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Ok => "OK",
            Self::Cancelled => "CANCELLED",
            Self::Unknown => "UNKNOWN",
            Self::InvalidArgument => "INVALID_ARGUMENT",
            Self::DeadlineExceeded => "DEADLINE_EXCEEDED",
            Self::NotFound => "NOT_FOUND",
            Self::AlreadyExists => "ALREADY_EXISTS",
            Self::PermissionDenied => "PERMISSION_DENIED",
            Self::ResourceExhausted => "RESOURCE_EXHAUSTED",
            Self::FailedPrecondition => "FAILED_PRECONDITION",
            Self::Aborted => "ABORTED",
            Self::OutOfRange => "OUT_OF_RANGE",
            Self::Unimplemented => "UNIMPLEMENTED",
            Self::Internal => "INTERNAL",
            Self::Unavailable => "UNAVAILABLE",
            Self::DataLoss => "DATA_LOSS",
            Self::Unauthenticated => "UNAUTHENTICATED",
        };
        f.write_str(name)
    }
}

/// How a call ended.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GrpcStatus {
    /// The code the server sent, or the one the failure maps to.
    pub code: GrpcStatusCode,
    /// The `grpc-message`, percent-decoded. Empty when the call succeeded.
    pub message: String,
    /// The trailers, minus `grpc-status` and `grpc-message`, which are this status.
    pub trailing_metadata: Metadata,
}

impl GrpcStatus {
    /// A status carrying nothing but a code and a reason.
    pub fn new(code: GrpcStatusCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            trailing_metadata: Metadata::new(),
        }
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

/// Percent-decoding for `grpc-message`, which is percent-encoded UTF-8 on the wire.
///
/// Invalid escapes are kept verbatim rather than rejected: this is a human-readable reason for a
/// failure that has already happened, and refusing to read it would replace the server's account
/// of the failure with an account of the encoding.
pub(crate) fn decode_message(raw: &[u8]) -> String {
    let mut out = Vec::with_capacity(raw.len());
    let mut index = 0;
    while index < raw.len() {
        match raw[index] {
            b'%' if index + 2 < raw.len() => {
                match (hex(raw[index + 1]), hex(raw[index + 2])) {
                    (Some(high), Some(low)) => {
                        out.push(high << 4 | low);
                        index += 3;
                    }
                    _ => {
                        out.push(raw[index]);
                        index += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_outside_the_defined_range_reads_as_unknown() {
        assert_eq!(GrpcStatusCode::from_wire(14), GrpcStatusCode::Unavailable);
        assert_eq!(GrpcStatusCode::from_wire(2), GrpcStatusCode::Unknown);
        assert_eq!(GrpcStatusCode::from_wire(99), GrpcStatusCode::Unknown);
        assert_eq!(GrpcStatusCode::from_wire(-1), GrpcStatusCode::Unknown);
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
}
