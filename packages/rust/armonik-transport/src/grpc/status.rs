use http::header::HeaderMap;

use tonic::Code;

use super::metadata::Metadata;
use super::origin::Origin;

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

/// How a request its peer's application never saw was not seen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Unprocessed {
    /// hyper dropped it before sending it, its connection closing under it.
    Unsent,
    /// The peer's HTTP/2 layer refused the stream, with REFUSED_STREAM.
    RefusedStream,
    /// The peer's GOAWAY left the stream unprocessed.
    GoAway,
}

impl Unprocessed {
    /// What an origin says of whether the peer's application saw the request.
    ///
    /// h2 gives a stream the peer's GOAWAY as its error only when the stream is past the last one
    /// that GOAWAY says it processes, or opened after it; a stream it processes ends on whatever
    /// closes the connection.
    pub(crate) fn of(origin: &Origin) -> Option<Self> {
        match origin {
            Origin::Unsent => Some(Self::Unsent),
            Origin::Reset(reason) if *reason == h2::Reason::REFUSED_STREAM => {
                Some(Self::RefusedStream)
            }
            Origin::GoAway => Some(Self::GoAway),
            _ => None,
        }
    }
}

impl std::fmt::Display for Unprocessed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unsent => "the request was never sent",
            Self::RefusedStream => "the peer refused the stream",
            Self::GoAway => "the peer's GOAWAY left the request unprocessed",
        })
    }
}

/// What a status that did not come from the server's trailers carries beside its code, as the
/// source of the `tonic::Status` it travels in: where the attempt ended, and whether its peer's
/// application ever saw the request.
#[derive(Clone, Debug)]
pub(crate) struct Failure {
    pub(crate) origin: Origin,
    pub(crate) unprocessed: Option<Unprocessed>,
}

impl Failure {
    pub(crate) fn of(origin: Origin) -> Self {
        Self {
            origin,
            unprocessed: None,
        }
    }

    /// The failure of a request hyper could not send or lost.
    pub(crate) fn of_request(error: &hyper::Error) -> Self {
        let origin = Origin::of_request(error);
        Self {
            unprocessed: Unprocessed::of(&origin),
            origin,
        }
    }

    pub(crate) fn marked(status: &tonic::Status) -> Option<Self> {
        std::error::Error::source(status)?
            .downcast_ref::<Self>()
            .cloned()
    }

    /// The status, in the type tonic's client carries it in, marked with this failure.
    pub(crate) fn on(self, status: GrpcStatus) -> tonic::Status {
        let mut status = tonic::Status::new(status.code, status.message);
        status.set_source(std::sync::Arc::new(self));
        status
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.origin.describe())?;
        match self.unprocessed {
            Some(unprocessed) => write!(f, ": {unprocessed}"),
            None => Ok(()),
        }
    }
}

impl std::error::Error for Failure {}

impl Origin {
    /// Where the loss of a request came from, read off the hyper error.
    ///
    /// The peer's GOAWAY is its own origin, apart from a reset: it ends every stream past the last
    /// one it processes, whatever the reason it gives. A reset is the peer's. What ends with no
    /// reason, or with a protocol error this side detected, is the connection.
    pub(crate) fn of_request(error: &hyper::Error) -> Self {
        if error.is_canceled() {
            return Self::Unsent;
        }
        match h2_error(error) {
            Some(h2) if h2.is_remote() && h2.is_go_away() => Self::GoAway,
            Some(h2) if h2.is_remote() => h2.reason().map_or(Self::Connection, Self::Reset),
            _ => Self::Connection,
        }
    }
}

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
        use super::{Origin, Unprocessed};
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
        let origin = Origin::of_request(&error);
        assert_eq!(origin, Origin::Unsent, "{error}");
        assert_eq!(
            Unprocessed::of(&origin),
            Some(Unprocessed::Unsent),
            "{error}"
        );
    }

    /// The failure rides as the source of the status tonic carries, and a status the engine did not
    /// mark has none.
    #[test]
    fn a_failure_travels_as_the_source_of_a_status() {
        use super::{Failure, GrpcStatus, GrpcStatusCode, Origin, Unprocessed};

        let status = Failure {
            origin: Origin::Dial,
            unprocessed: Some(Unprocessed::GoAway),
        }
        .on(GrpcStatus::new(GrpcStatusCode::Unavailable, "no dial"));

        let read = Failure::marked(&status).expect("marked");
        assert_eq!(read.origin, Origin::Dial);
        assert_eq!(read.unprocessed, Some(Unprocessed::GoAway));
        assert_eq!(status.code(), tonic::Code::Unavailable);
        assert!(Failure::marked(&tonic::Status::unavailable("no mark")).is_none());
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
