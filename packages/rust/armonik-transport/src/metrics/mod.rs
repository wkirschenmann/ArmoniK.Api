//! What the engine counts, and what it reads of its own state, for a host to observe.
//!
//! The counters are kept by the `metrics` feature. Without it every counting point is an empty
//! function, the registry has no state, and [`Metrics::stats`] answers an empty [`Stats`], so a
//! caller never has to know which build it runs against.
//!
//! A counter is written by one task: a call's request body, its driver, or its connection's task,
//! each in a block that task owns, and a read sums those blocks with what finished calls and
//! connections left in a shard of the registry. What happens once or rarely per call - a retry, a
//! resend, a refusal - is added to a shard's totals under its lock.

use crate::grpc::GrpcStatusCode;

#[cfg(feature = "metrics")]
mod on;
#[cfg(feature = "metrics")]
use on as imp;

#[cfg(not(feature = "metrics"))]
mod off;
#[cfg(not(feature = "metrics"))]
use off as imp;

pub use imp::Metrics;
pub(crate) use imp::{CallCounters, CallGuard, ConnCounters, Sniffer};

/// The slots of [`Stats::calls_ended`]: a gRPC status is its own number, 0 to 16.
pub const STATUS_SLOTS: usize = 17;

/// The slots of [`Stats::retries`]: the first 16 for the statuses 1 to 16 (OK is never retried),
/// then the HTTP slots for 408, 429, 500, 502, 503, 504 and any other status, the reset slots
/// for HTTP/2 error codes 0 to 13 and any other, then a pushback, a dial and a connection.
pub const RETRY_SLOTS: usize = 41;

/// The first slot of [`Stats::retries`] that is an HTTP status, and how many there are.
pub const RETRY_HTTP_AT: usize = 16;
pub const RETRY_HTTP_SLOTS: usize = 7;
/// The HTTP statuses of the slots from [`RETRY_HTTP_AT`], before the slot for any other.
pub const RETRY_HTTP_STATUSES: [u16; 6] = [408, 429, 500, 502, 503, 504];
/// The first slot of [`Stats::retries`] that is a reset, and how many there are: one per HTTP/2
/// error code RFC 9113 lists and one for any other.
pub const RETRY_RESET_AT: usize = 23;
pub const RESET_SLOTS: usize = 15;
pub const RETRY_PUSHBACK: usize = 38;
pub const RETRY_DIAL: usize = 39;
pub const RETRY_CONNECTION: usize = 40;

/// The slots of [`Stats::connections_closed`], by [`CloseReason`].
pub const CLOSE_SLOTS: usize = 8;

/// Why an HTTP/2 session ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CloseReason {
    /// The peer sent a GOAWAY.
    GoAway = 0,
    /// The keepalive's PING went unanswered for its timeout.
    KeepaliveTimeout = 1,
    /// The engine closed it after it had no call for the idle timeout.
    IdleTimeout = 2,
    /// The connection failed reading or writing.
    IoError = 3,
    /// The engine closed it because its channel closed.
    LocalClose = 4,
    /// The peer closed the stream of bytes with no GOAWAY.
    PeerClosed = 5,
    /// A violation of HTTP/2, found by this side or reported by the peer.
    ProtocolError = 6,
    /// None of the above.
    Other = 7,
}

/// What the embedding crate of the engine reports for it to count.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum HostEvent {
    /// A delivery found every credit of the receive window spent.
    WindowWait,
    /// A read was held back, or a send made to wait, by the memory ceiling.
    MemoryWait,
    /// A send was refused, or a received message dropped, by the memory ceiling.
    MemoryRefusal,
}

/// The counters and gauges of a registry at the moment it was read.
///
/// Empty - `counting` false and every field zero - when the engine is built without the `metrics`
/// feature.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Stats {
    /// Whether this build keeps counters.
    pub counting: bool,
    pub calls_started: u64,
    /// Calls ended, by the number of the status they ended with.
    pub calls_ended: [u64; STATUS_SLOTS],
    pub messages_sent: u64,
    pub messages_received: u64,
    /// Retries, by the failure that was retried; see [`RETRY_SLOTS`].
    pub retries: [u64; RETRY_SLOTS],
    /// Retries the adaptive estimate stopped.
    pub retries_refused: u64,
    /// Calls whose messages outgrew a replay ceiling, so that they are not tried again.
    pub calls_not_replayable: u64,
    /// Requests the peer never processed, sent again at once.
    pub resends: u64,
    pub dials_tried: u64,
    pub dials_succeeded: u64,
    pub dials_failed: u64,
    /// Sessions closed, by [`CloseReason`].
    pub connections_closed: [u64; CLOSE_SLOTS],
    /// Streams the peer reset, by HTTP/2 error code; the last slot is any other code.
    pub streams_reset: [u64; RESET_SLOTS],
    /// HTTP/2 bytes written to and read from the connections, above TLS.
    pub wire_bytes_sent: u64,
    pub wire_bytes_received: u64,
    /// Message bytes as the caller wrote them, and as the engine sent them, a message once
    /// whatever its attempts, the gRPC prefix left out.
    pub message_bytes_raw: u64,
    pub message_bytes_sent: u64,
    pub host_window_waits: u64,
    pub host_memory_waits: u64,
    pub host_memory_refusals: u64,
    /// The rate of first attempts the capped channels allow together, a second; zero when none is
    /// capped.
    pub throttle_cap_per_second: f64,
    pub channels_capped: u64,
    /// Channels whose estimate has stopped retries.
    pub channels_retries_closed: u64,
    /// Calls waiting for their turn at a channel's cap.
    pub calls_waiting_at_cap: u64,
    /// Calls waiting for a session to open or to have room.
    pub calls_waiting_for_stream: u64,
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            counting: false,
            calls_started: 0,
            calls_ended: [0; STATUS_SLOTS],
            messages_sent: 0,
            messages_received: 0,
            retries: [0; RETRY_SLOTS],
            retries_refused: 0,
            calls_not_replayable: 0,
            resends: 0,
            dials_tried: 0,
            dials_succeeded: 0,
            dials_failed: 0,
            connections_closed: [0; CLOSE_SLOTS],
            streams_reset: [0; RESET_SLOTS],
            wire_bytes_sent: 0,
            wire_bytes_received: 0,
            message_bytes_raw: 0,
            message_bytes_sent: 0,
            host_window_waits: 0,
            host_memory_waits: 0,
            host_memory_refusals: 0,
            throttle_cap_per_second: 0.0,
            channels_capped: 0,
            channels_retries_closed: 0,
            calls_waiting_at_cap: 0,
            calls_waiting_for_stream: 0,
        }
    }
}

impl Stats {
    /// Calls ended with `code`.
    pub fn ended_with(&self, code: GrpcStatusCode) -> u64 {
        self.calls_ended[code as usize]
    }
}

/// What a channel reports of itself when the registry is read.
#[cfg_attr(not(feature = "metrics"), allow(dead_code))]
pub(crate) struct ChannelGauges {
    /// The rate its estimate caps first attempts at, when it does.
    pub(crate) cap: Option<f64>,
    pub(crate) retries_open: bool,
    pub(crate) waiting_at_cap: u64,
}

/// A channel, as the registry reads its gauges.
#[cfg_attr(not(feature = "metrics"), allow(dead_code))]
pub(crate) trait GaugeSource: Send + Sync {
    fn gauges(&self) -> ChannelGauges;
}

/// The slot of the HTTP/2 error code `code` among [`RESET_SLOTS`]: its number, or the last slot.
pub(crate) fn reset_slot(code: u32) -> usize {
    if (code as usize) < RESET_SLOTS - 1 {
        code as usize
    } else {
        RESET_SLOTS - 1
    }
}
