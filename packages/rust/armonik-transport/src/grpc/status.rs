
use http::header::{HeaderMap, CONTENT_TYPE};
use http::StatusCode;

use super::metadata::{Metadata, GRPC_MESSAGE, GRPC_STATUS};

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

    pub fn from_http_status(status: u16) -> Self {
        match status {
            400 => Self::Internal,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GrpcStatus {
    pub code: GrpcStatusCode,
    pub message: String,
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

pub(crate) fn cancelled() -> GrpcStatus {
    GrpcStatus::new(GrpcStatusCode::Cancelled, "the call was cancelled")
}

pub(crate) fn stated_status(headers: &HeaderMap) -> Option<GrpcStatus> {
    let raw = headers.get(GRPC_STATUS)?;

    let code = std::str::from_utf8(raw.as_bytes())
        .ok()
        .and_then(|text| text.trim().parse::<i32>().ok());

    let Some(code) = code else {
        return Some(GrpcStatus::new(
            GrpcStatusCode::Internal,
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

pub(crate) fn http_status(status: StatusCode, headers: &HeaderMap) -> GrpcStatus {
    GrpcStatus {
        code: GrpcStatusCode::from_http_status(status.as_u16()),
        message: format!(
            "the peer answered HTTP {} rather than gRPC",
            status.as_u16()
        ),
        trailing_metadata: Metadata::from_headers(headers),
    }
}

pub(crate) fn speaks_grpc(headers: &HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            let value = value.trim().to_ascii_lowercase();
            value
                .strip_prefix("application/grpc")
                .map(|rest| rest.is_empty() || rest.starts_with(['+', ';']))
        })
        .unwrap_or(false)
}

fn decode_message(raw: &[u8]) -> String {
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
        assert_eq!(GrpcStatusCode::from_wire(14), GrpcStatusCode::Unavailable);
        assert_eq!(GrpcStatusCode::from_wire(2), GrpcStatusCode::Unknown);
        assert_eq!(GrpcStatusCode::from_wire(99), GrpcStatusCode::Unknown);
        assert_eq!(GrpcStatusCode::from_wire(-1), GrpcStatusCode::Unknown);
    }

    #[test]
    fn a_percent_escape_is_decoded_and_a_broken_one_is_kept() {
        assert_eq!(decode_message(b"plain"), "plain");
        assert_eq!(decode_message(b"a%20b"), "a b");
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

        assert_eq!(status.code, GrpcStatusCode::FailedPrecondition);
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
            GrpcStatusCode::Internal
        );
    }

    #[test]
    fn a_status_without_a_message_is_still_a_status() {
        let status = stated_status(&headers(&[("grpc-status", "0")])).expect("a status");
        assert_eq!(status.code, GrpcStatusCode::Ok);
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
            GrpcStatusCode::Unimplemented
        );
        assert_eq!(
            http_status(StatusCode::SERVICE_UNAVAILABLE, &HeaderMap::new()).code,
            GrpcStatusCode::Unavailable
        );
        assert_eq!(
            http_status(StatusCode::IM_A_TEAPOT, &HeaderMap::new()).code,
            GrpcStatusCode::Unknown
        );
    }
}
