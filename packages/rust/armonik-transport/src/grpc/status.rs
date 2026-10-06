use http::header::HeaderMap;

use tonic::Code;

use super::metadata::Metadata;

/// The code a gRPC status carries. `tonic::Code` is that set, and redeclaring it here would only
/// be a second spelling of the same seventeen values.
pub type GrpcStatusCode = Code;

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

    pub(crate) fn ok(trailers: &HeaderMap) -> Self {
        Self {
            code: Code::Ok,
            message: String::new(),
            trailing_metadata: Metadata::from_headers(trailers),
        }
    }

    pub fn cancelled() -> Self {
        Self::new(GrpcStatusCode::Cancelled, "the call was cancelled")
    }

    pub(crate) fn deadline_exceeded() -> Self {
        Self::new(
            GrpcStatusCode::DeadlineExceeded,
            "the call's deadline passed before it ended",
        )
    }

    pub(crate) fn unreachable(error: impl std::fmt::Display) -> Self {
        Self::new(GrpcStatusCode::Unavailable, error.to_string())
    }

    pub(crate) fn request_lost(error: &hyper::Error) -> Self {
        Self::new(
            reset_code(error),
            format!("the request did not reach the peer: {}", described(error)),
        )
    }

    pub(crate) fn stream_broke(error: &hyper::Error) -> Self {
        Self::new(
            reset_code(error),
            format!("the response stream broke: {}", described(error)),
        )
    }
}

impl From<tonic::Status> for GrpcStatus {
    fn from(mut status: tonic::Status) -> Self {
        let trailers = std::mem::take(status.metadata_mut()).into_headers();
        Self {
            code: status.code(),
            message: status.message().to_owned(),
            trailing_metadata: Metadata::from_headers(&trailers),
        }
    }
}

/// What a broken stream means, from the reason of the h2 error behind it.
///
/// gRPC's own table, in PROTOCOL-HTTP2. Reporting every reason as UNAVAILABLE would tell a host
/// that retries on it to repeat a call the peer deliberately cancelled, and to keep repeating one
/// that failed on a framing error that is never transient.
///
/// UNAVAILABLE stays the answer for everything that is not a reset - an I/O error, a connection
/// that died or that a GOAWAY ended, a peer that never answered - and for REFUSED_STREAM, which is
/// the one reason that does mean "try again".
fn reset_code(error: &hyper::Error) -> GrpcStatusCode {
    code_of(reset_reason(error))
}

fn code_of(reason: Option<h2::Reason>) -> GrpcStatusCode {
    let Some(reason) = reason else {
        return GrpcStatusCode::Unavailable;
    };
    // Compared rather than matched: the values of h2::Reason are associated constants, and a
    // constant is only a pattern when its type opts into structural matching.
    if reason == h2::Reason::CANCEL {
        GrpcStatusCode::Cancelled
    } else if reason == h2::Reason::ENHANCE_YOUR_CALM {
        GrpcStatusCode::ResourceExhausted
    } else if reason == h2::Reason::INADEQUATE_SECURITY {
        GrpcStatusCode::PermissionDenied
    } else if reason == h2::Reason::REFUSED_STREAM {
        GrpcStatusCode::Unavailable
    } else {
        // Every framing and protocol error, and NO_ERROR, which on a stream that owed a status
        // means the peer ended without giving one.
        GrpcStatusCode::Internal
    }
}

/// None for a GOAWAY the peer sent, whose reason is the connection's: gRPC answers UNAVAILABLE
/// for a stream a peer's GOAWAY ended, whatever the reason. A GOAWAY h2 sent itself keeps it.
fn reset_reason(error: &hyper::Error) -> Option<h2::Reason> {
    let h2 = h2_error(error)?;
    h2.reason().filter(|_| !(h2.is_go_away() && h2.is_remote()))
}

/// The source of a status whose request the peer's application never saw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Unprocessed {
    /// hyper dropped it before sending it, its connection closing under it.
    Unsent,
    /// The peer's HTTP/2 layer refused the stream, or its GOAWAY left the stream unprocessed.
    Refused,
}

impl Unprocessed {
    pub(crate) fn of(error: &hyper::Error) -> Option<Self> {
        if error.is_canceled() {
            return Some(Self::Unsent);
        }
        let h2 = h2_error(error)?;
        let refused = h2.is_reset() && h2.reason() == Some(h2::Reason::REFUSED_STREAM);
        // h2 gives a stream the peer's GOAWAY as its error only when the stream is past the last
        // one that GOAWAY says it processes, or opened after it; a stream it processes ends on
        // whatever closes the connection.
        (h2.is_remote() && (h2.is_go_away() || refused)).then_some(Self::Refused)
    }

    pub(crate) fn marked(status: &tonic::Status) -> Option<Self> {
        std::error::Error::source(status)?
            .downcast_ref::<Self>()
            .copied()
    }
}

impl std::fmt::Display for Unprocessed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unsent => "the request was never sent",
            Self::Refused => "the peer did not process the request",
        })
    }
}

impl std::error::Error for Unprocessed {}

/// hyper's error and the h2 error behind it: hyper's own says only "http2 error", and a reason
/// the code does not carry is read nowhere else.
fn described(error: &hyper::Error) -> String {
    match h2_error(error) {
        Some(h2) => format!("{error}: {h2}"),
        None => error.to_string(),
    }
}

/// The `h2::Error` behind a hyper error, if that is what it is.
///
/// Down the source chain rather than off the error itself: hyper wraps it and exposes neither the
/// type nor the reason. Which means this finds one only when hyper linked the same `h2` this
/// crate names - see the note on the workspace dependency.
fn h2_error(error: &hyper::Error) -> Option<&h2::Error> {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(cause) = source {
        if let Some(h2) = cause.downcast_ref::<h2::Error>() {
            return Some(h2);
        }
        source = cause.source();
    }
    None
}

impl std::error::Error for GrpcStatus {}

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

#[cfg(test)]
mod tests {
    /// A request handed to a connection that is gone never left this side, which hyper reports as
    /// cancelled.
    #[tokio::test]
    async fn a_request_no_connection_took_is_unsent() {
        use super::Unprocessed;
        use hyper_util::rt::{TokioExecutor, TokioIo};

        let (io, _peer) = tokio::io::duplex(4096);
        let (mut sender, connection) =
            hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(io))
                .await
                .expect("a handshake");
        drop(connection);
        let error = sender
            .send_request(http::Request::new(
                http_body_util::Empty::<bytes::Bytes>::new(),
            ))
            .await
            .expect_err("no connection to take it");
        assert_eq!(
            Unprocessed::of(&error),
            Some(Unprocessed::Unsent),
            "{error}"
        );
    }

    /// The table, reason by reason, without a `hyper::Error` - which has no public constructor,
    /// so the downcast that feeds this is `grpc_unary.rs`'s to check.
    #[test]
    fn every_reset_reason_gets_the_code_grpc_gives_it() {
        use super::{code_of, GrpcStatusCode};

        assert_eq!(
            code_of(Some(h2::Reason::CANCEL)),
            GrpcStatusCode::Cancelled,
            "a peer that abandoned the call is not a peer that is unreachable"
        );
        assert_eq!(
            code_of(Some(h2::Reason::ENHANCE_YOUR_CALM)),
            GrpcStatusCode::ResourceExhausted
        );
        assert_eq!(
            code_of(Some(h2::Reason::INADEQUATE_SECURITY)),
            GrpcStatusCode::PermissionDenied
        );
        assert_eq!(
            code_of(Some(h2::Reason::REFUSED_STREAM)),
            GrpcStatusCode::Unavailable,
            "the one reason that does mean try again"
        );

        for framing in [
            h2::Reason::NO_ERROR,
            h2::Reason::PROTOCOL_ERROR,
            h2::Reason::INTERNAL_ERROR,
            h2::Reason::FLOW_CONTROL_ERROR,
            h2::Reason::SETTINGS_TIMEOUT,
            h2::Reason::STREAM_CLOSED,
            h2::Reason::FRAME_SIZE_ERROR,
            h2::Reason::COMPRESSION_ERROR,
            h2::Reason::CONNECT_ERROR,
        ] {
            assert_eq!(
                code_of(Some(framing)),
                GrpcStatusCode::Internal,
                "{framing:?}"
            );
        }

        assert_eq!(
            code_of(None),
            GrpcStatusCode::Unavailable,
            "an I/O error or a dead connection is not a reset"
        );
    }
}
