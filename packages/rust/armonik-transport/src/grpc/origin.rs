//! Where the end of an attempt came from.
//!
//! A status carries a code, and the engine maps several origins onto one code: UNAVAILABLE is the
//! server's own, a proxy's 503, a dial that failed, a stream a GOAWAY ended and a stream refused.
//! So each failed attempt carries its origin beside its status, and the pushback its server stated,
//! which tells these apart.

use std::time::Duration;

use http::HeaderMap;

/// The trailer in which the server says how long to wait before a retry, or not to retry.
const PUSHBACK: &str = "grpc-retry-pushback-ms";

/// What ended an attempt, as far as the engine can tell.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Origin {
    /// The server's own status, in its trailers or in a Trailers-Only head: `OK`, an application
    /// status, or a refusal the server chose to give.
    Server,
    /// An answer that states no gRPC status, from a proxy or a gateway: an HTTP error, or HTTP 200
    /// without a gRPC content type.
    Http(http::StatusCode),
    /// The peer reset the stream before its response head came, for this reason.
    Reset(h2::Reason),
    /// The peer's GOAWAY ended the stream.
    GoAway,
    /// The connection ended under the call, before the response head, with no reset and no GOAWAY
    /// from the peer: an I/O error, a connection closed, a keepalive that timed out, or a protocol
    /// error this side detected.
    Connection,
    /// The dial failed: the connection, the TLS handshake or the time allowed for them.
    Dial,
    /// hyper dropped the request before sending it, its connection closing under it.
    Unsent,
    /// The stream broke after the response head arrived.
    Broke,
    /// The engine's own: a cancel, a message over a limit, a request over the header-list limit,
    /// malformed trailers or messages, a closed channel.
    Local,
}

impl Origin {
    /// In words, for a log line.
    pub(crate) fn describe(&self) -> String {
        match self {
            Self::Server => "the server's status".to_owned(),
            Self::Http(status) => format!("an HTTP {} that states no status", status.as_u16()),
            Self::Reset(reason) => format!("a reset, {reason}"),
            Self::GoAway => "the peer's GOAWAY".to_owned(),
            Self::Connection => "the connection ending".to_owned(),
            Self::Dial => "a dial that failed".to_owned(),
            Self::Unsent => "a request that was never sent".to_owned(),
            Self::Broke => "a stream that broke after its head".to_owned(),
            Self::Local => "the engine".to_owned(),
        }
    }
}

/// What a failed attempt's server said of a retry, in `grpc-retry-pushback-ms`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Pushback {
    /// Nothing: the backoff decides.
    Unsaid,
    /// Retry after this long.
    After(Duration),
    /// Do not retry: a negative or unreadable value, which gRFC A6 reads so.
    Refused,
}

impl Pushback {
    pub(crate) fn of(headers: &HeaderMap) -> Self {
        let Some(value) = headers.get(PUSHBACK) else {
            return Self::Unsaid;
        };
        match value
            .to_str()
            .ok()
            .and_then(|text| text.parse::<u64>().ok())
        {
            Some(millis) => Self::After(Duration::from_millis(millis)),
            None => Self::Refused,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn said(value: &str) -> Pushback {
        let mut headers = HeaderMap::new();
        headers.insert(PUSHBACK, value.parse().expect("a header value"));
        Pushback::of(&headers)
    }

    #[test]
    fn a_pushback_is_a_wait_or_a_refusal_or_nothing() {
        assert_eq!(Pushback::of(&HeaderMap::new()), Pushback::Unsaid);
        assert_eq!(said("250"), Pushback::After(Duration::from_millis(250)));
        assert_eq!(said("0"), Pushback::After(Duration::ZERO));
        assert_eq!(said("-1"), Pushback::Refused);
        assert_eq!(said("soon"), Pushback::Refused);
    }
}
