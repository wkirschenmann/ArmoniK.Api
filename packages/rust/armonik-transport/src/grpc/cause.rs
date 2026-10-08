//! What can end an attempt, as a list of failures names it.
//!
//! A retry policy and the adaptive estimate each take a list of causes as data, so that what is
//! retried, what is transient and what is overload are the caller's to state and none is a preset.

use std::str::FromStr;

use super::origin::{Origin, Pushback};
use super::status::GrpcStatusCode;

/// One kind of failure, which a list names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Cause {
    /// A gRPC status in the server's trailers, `OK` excepted.
    Status(GrpcStatusCode),
    /// An HTTP status that a proxy or a gateway answered with, and no gRPC status.
    Http(u16),
    /// A reset of the stream by the peer before the response head, by the number of its HTTP/2
    /// error code, which RFC 9113 lists.
    Reset(u32),
    /// A pushback in `grpc-retry-pushback-ms` that asks for a wait, on any failed attempt.
    Pushback,
    /// A dial that failed: the connection, the TLS handshake or the time allowed for them.
    Dial,
    /// A connection that ended under the call before the response head, with no reset. A retry
    /// policy also reads a GOAWAY that left the call unprocessed, and a request hyper dropped
    /// unsent, as the connection's end.
    Connection,
}

const STATUSES: [(&str, GrpcStatusCode); 16] = [
    ("CANCELLED", GrpcStatusCode::Cancelled),
    ("UNKNOWN", GrpcStatusCode::Unknown),
    ("INVALID_ARGUMENT", GrpcStatusCode::InvalidArgument),
    ("DEADLINE_EXCEEDED", GrpcStatusCode::DeadlineExceeded),
    ("NOT_FOUND", GrpcStatusCode::NotFound),
    ("ALREADY_EXISTS", GrpcStatusCode::AlreadyExists),
    ("PERMISSION_DENIED", GrpcStatusCode::PermissionDenied),
    ("RESOURCE_EXHAUSTED", GrpcStatusCode::ResourceExhausted),
    ("FAILED_PRECONDITION", GrpcStatusCode::FailedPrecondition),
    ("ABORTED", GrpcStatusCode::Aborted),
    ("OUT_OF_RANGE", GrpcStatusCode::OutOfRange),
    ("UNIMPLEMENTED", GrpcStatusCode::Unimplemented),
    ("INTERNAL", GrpcStatusCode::Internal),
    ("UNAVAILABLE", GrpcStatusCode::Unavailable),
    ("DATA_LOSS", GrpcStatusCode::DataLoss),
    ("UNAUTHENTICATED", GrpcStatusCode::Unauthenticated),
];

/// The HTTP/2 error codes of RFC 9113, section 7, by their number.
const RESETS: [&str; 14] = [
    "NO_ERROR",
    "PROTOCOL_ERROR",
    "INTERNAL_ERROR",
    "FLOW_CONTROL_ERROR",
    "SETTINGS_TIMEOUT",
    "STREAM_CLOSED",
    "FRAME_SIZE_ERROR",
    "REFUSED_STREAM",
    "CANCEL",
    "COMPRESSION_ERROR",
    "CONNECT_ERROR",
    "ENHANCE_YOUR_CALM",
    "INADEQUATE_SECURITY",
    "HTTP_1_1_REQUIRED",
];

impl std::fmt::Display for Cause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Status(code) => match STATUSES.iter().find(|(_, listed)| listed == code) {
                Some((name, _)) => write!(f, "Status.{name}"),
                None => write!(f, "Status.{code:?}"),
            },
            Self::Http(status) => write!(f, "Http.{status}"),
            Self::Reset(reason) => match RESETS.get(*reason as usize) {
                Some(name) => write!(f, "Reset.{name}"),
                None => write!(f, "Reset.{reason}"),
            },
            Self::Pushback => f.write_str("Pushback"),
            Self::Dial => f.write_str("Dial"),
            Self::Connection => f.write_str("Connection"),
        }
    }
}

/// What a list entry that names no cause says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownCause(pub String);

impl std::fmt::Display for UnknownCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "`{}` names no failure: an entry is `Status.` and a gRPC status such as UNAVAILABLE, \
             `Http.` and a status from 100 to 599, `Reset.` and an HTTP/2 error code such as \
             ENHANCE_YOUR_CALM, or `Pushback`, `Dial` or `Connection`",
            self.0
        )
    }
}

impl std::error::Error for UnknownCause {}

impl FromStr for Cause {
    type Err = UnknownCause;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let unknown = || UnknownCause(text.to_owned());
        match text {
            "Pushback" => return Ok(Self::Pushback),
            "Dial" => return Ok(Self::Dial),
            "Connection" => return Ok(Self::Connection),
            _ => {}
        }
        let (kind, name) = text.split_once('.').ok_or_else(unknown)?;
        match kind {
            "Status" => STATUSES
                .iter()
                .find(|(listed, _)| *listed == name)
                .map(|(_, code)| Self::Status(*code))
                .ok_or_else(unknown),
            "Http" => {
                let digits = !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_digit());
                match name.parse::<u16>() {
                    Ok(status) if digits && (100..=599).contains(&status) => Ok(Self::Http(status)),
                    _ => Err(unknown()),
                }
            }
            "Reset" => RESETS
                .iter()
                .position(|listed| *listed == name)
                .map(|number| Self::Reset(number as u32))
                .ok_or_else(unknown),
            _ => Err(unknown()),
        }
    }
}

impl Cause {
    /// What ended an attempt, as a cause, or none for what no list can name: a success, a GOAWAY, a
    /// request that was never sent, a stream that broke after its head, what the engine ended
    /// itself, and an HTTP 200 that states no gRPC status.
    pub(crate) fn of(origin: &Origin, code: GrpcStatusCode) -> Option<Self> {
        match origin {
            Origin::GoAway | Origin::Unsent | Origin::Broke | Origin::Local => None,
            Origin::Server => (code != GrpcStatusCode::Ok).then_some(Self::Status(code)),
            Origin::Http(status) if status.as_u16() == 200 => None,
            Origin::Http(status) => Some(Self::Http(status.as_u16())),
            Origin::Reset(reason) => Some(Self::Reset(u32::from(*reason))),
            Origin::Connection => Some(Self::Connection),
            Origin::Dial => Some(Self::Dial),
        }
    }
}

/// Whether `list` names the failure that ended an attempt: its cause, or a pushback that asks for a
/// wait, whatever the cause, since the server then rations capacity.
pub(crate) fn names(
    list: &[Cause],
    origin: &Origin,
    code: GrpcStatusCode,
    pushback: Pushback,
) -> bool {
    let Some(cause) = Cause::of(origin, code) else {
        return false;
    };
    list.contains(&cause)
        || (matches!(pushback, Pushback::After(_)) && list.contains(&Cause::Pushback))
}

/// Whether a retry list names the failure that ended an attempt. As [`names`], and a GOAWAY that
/// left the call unprocessed and a request that was never sent count as the connection's: a call
/// goes again once for each of them whatever the policy, and a further one is the policy's.
pub(crate) fn retried(
    list: &[Cause],
    origin: &Origin,
    code: GrpcStatusCode,
    pushback: Pushback,
) -> bool {
    match origin {
        Origin::GoAway | Origin::Unsent => list.contains(&Cause::Connection),
        _ => names(list, origin, code, pushback),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_cause_reads_back_as_it_is_written() {
        let mut causes = vec![Cause::Pushback, Cause::Dial, Cause::Connection];
        causes.extend(STATUSES.iter().map(|(_, code)| Cause::Status(*code)));
        causes.extend((0..14).map(Cause::Reset));
        causes.extend([100, 429, 503, 599].map(Cause::Http));
        for cause in causes {
            assert_eq!(cause.to_string().parse::<Cause>(), Ok(cause));
        }
        assert_eq!(
            "Reset.ENHANCE_YOUR_CALM".parse::<Cause>(),
            Ok(Cause::Reset(11))
        );
        assert_eq!(
            "Status.UNAVAILABLE".parse::<Cause>(),
            Ok(Cause::Status(GrpcStatusCode::Unavailable))
        );
    }

    #[test]
    fn an_entry_that_names_no_failure_is_refused() {
        for text in [
            "",
            "Status",
            "Status.",
            "Status.OK",
            "Status.unavailable",
            "Status.UNAVAILABLE ",
            "Http.99",
            "Http.600",
            "Http.+503",
            "Http.5o3",
            "Http.",
            "Reset.11",
            "Reset.NO_SUCH",
            "Dial.x",
            "pushback",
            "Status.UNAVAILABLE.x",
            "Other.1",
        ] {
            assert!(text.parse::<Cause>().is_err(), "{text:?}");
        }
    }

    #[test]
    fn what_no_list_can_name_has_no_cause() {
        let code = GrpcStatusCode::Unavailable;
        for origin in [Origin::GoAway, Origin::Unsent, Origin::Broke, Origin::Local] {
            assert_eq!(Cause::of(&origin, code), None, "{origin:?}");
        }
        assert_eq!(Cause::of(&Origin::Server, GrpcStatusCode::Ok), None);
        assert_eq!(
            Cause::of(&Origin::Http(http::StatusCode::OK), GrpcStatusCode::Unknown),
            None
        );
        assert_eq!(Cause::of(&Origin::Server, code), Some(Cause::Status(code)));
    }
}
