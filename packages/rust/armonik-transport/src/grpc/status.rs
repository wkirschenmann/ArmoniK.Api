use http::header::{HeaderMap, CONTENT_TYPE};
use http::StatusCode;

use tonic::Code;

use super::metadata::Metadata;

pub(crate) const GRPC_STATUS: &str = "grpc-status";
pub(crate) const GRPC_MESSAGE: &str = "grpc-message";

/// The code a gRPC status carries. `tonic::Code` is that set, and redeclaring it here would only
/// be a second spelling of the same seventeen values.
pub type GrpcStatusCode = Code;

fn from_http_status(status: u16) -> GrpcStatusCode {
    match status {
        400 => Code::Internal,
        401 => Code::Unauthenticated,
        403 => Code::PermissionDenied,
        404 => Code::Unimplemented,
        429 | 502 | 503 | 504 => Code::Unavailable,
        _ => Code::Unknown,
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

    pub fn cancelled() -> Self {
        Self::new(GrpcStatusCode::Cancelled, "the call was cancelled")
    }

    pub(crate) fn unreachable(error: impl std::fmt::Display) -> Self {
        Self::new(GrpcStatusCode::Unavailable, error.to_string())
    }

    pub(crate) fn request_lost(error: impl std::fmt::Display) -> Self {
        Self::new(
            GrpcStatusCode::Unavailable,
            format!("the request did not reach the peer: {error}"),
        )
    }

    pub(crate) fn stream_broke(error: impl std::fmt::Display) -> Self {
        Self::new(
            GrpcStatusCode::Unavailable,
            format!("the response stream broke: {error}"),
        )
    }

    pub(crate) fn not_grpc() -> Self {
        Self::new(
            GrpcStatusCode::Internal,
            "the peer answered HTTP 200 without a gRPC content type",
        )
    }

    pub(crate) fn no_status() -> Self {
        Self::new(
            GrpcStatusCode::Internal,
            "the peer ended the stream without a grpc-status",
        )
    }

    pub(crate) fn no_trailing_status() -> Self {
        Self::new(
            GrpcStatusCode::Internal,
            "the peer's trailers carry no grpc-status",
        )
    }

    pub(crate) fn ended_mid_message() -> Self {
        Self::new(
            GrpcStatusCode::Internal,
            "the peer ended the stream in the middle of a message",
        )
    }
}

impl std::fmt::Display for GrpcStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Debug, not Display: tonic's Display for a code is a sentence, and what a reader of a
        // status wants first is the name.
        if self.message.is_empty() {
            write!(f, "{:?}", self.code)
        } else {
            write!(f, "{:?}: {}", self.code, self.message)
        }
    }
}

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

pub(crate) fn stated_status(headers: &HeaderMap) -> Option<GrpcStatus> {
    let raw = headers.get(GRPC_STATUS)?;

    let code = raw
        .to_str()
        .ok()
        .and_then(|text| text.trim().parse::<i32>().ok());

    let Some(code) = code else {
        return Some(GrpcStatus::new(
            GrpcStatusCode::Internal,
            "the peer's grpc-status is not a number",
        ));
    };

    Some(GrpcStatus {
        code: Code::from_i32(code),
        message: headers
            .get(GRPC_MESSAGE)
            .map(|value| decode_message(value.as_bytes()))
            .unwrap_or_default(),
        trailing_metadata: Metadata::from_headers(headers),
    })
}

fn http_status(status: StatusCode, headers: &HeaderMap) -> GrpcStatus {
    GrpcStatus {
        code: from_http_status(status.as_u16()),
        message: format!(
            "the peer answered HTTP {} rather than gRPC",
            status.as_u16()
        ),
        trailing_metadata: Metadata::from_headers(headers),
    }
}

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

fn decode_message(raw: &[u8]) -> String {
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
