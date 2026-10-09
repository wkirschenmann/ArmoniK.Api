//! The registry of a build without the `metrics` feature: nothing is kept, and each counting
//! point is an empty function the compiler removes.

use std::sync::Weak;

use super::{CloseReason, GaugeSource, HostEvent, Stats};
use crate::grpc::GrpcStatusCode;

/// The registry. It holds nothing.
#[derive(Clone, Debug, Default)]
pub struct Metrics;

impl Metrics {
    pub fn new() -> Self {
        Self
    }

    /// An empty [`Stats`]: this build keeps no counter.
    pub fn stats(&self) -> Stats {
        Stats::default()
    }

    pub fn count_host(&self, _event: HostEvent) {}

    pub(crate) fn watch(&self, _source: Weak<dyn GaugeSource>) {}

    pub(crate) fn start_call(&self, _counters: &CallCounters) -> CallGuard {
        CallGuard
    }

    pub(crate) fn retry(&self, _slot: impl FnOnce() -> usize) {}

    pub(crate) fn retry_refused(&self) {}

    pub(crate) fn not_replayable(&self) {}

    pub(crate) fn resend(&self) {}

    pub(crate) fn dial_tried(&self) {}

    pub(crate) fn dial_failed(&self, _conn: &ConnCounters) {}

    pub(crate) fn connection_open(&self, _conn: &ConnCounters) -> ConnGuard {
        ConnGuard
    }
}

/// A call's counters.
#[derive(Clone, Debug, Default)]
pub(crate) struct CallCounters;

impl CallCounters {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn message_received(&self) {}

    pub(crate) fn message_sent(&self, _raw: usize, _sent: usize) {}

    pub(crate) fn window_wait(&self) {}

    pub(crate) fn wait_for_stream(&self) -> StreamWait<'_> {
        StreamWait(std::marker::PhantomData)
    }
}

/// A call's wait for a stream, which counts while it is held.
pub(crate) struct StreamWait<'a>(std::marker::PhantomData<&'a ()>);

/// A call's place in the registry.
pub(crate) struct CallGuard;

impl CallGuard {
    pub(crate) fn end(self, _code: GrpcStatusCode) {}
}

/// A connection's counters.
#[derive(Clone, Debug, Default)]
pub(crate) struct ConnCounters;

impl ConnCounters {
    pub(crate) fn new() -> Self {
        Self
    }

    /// Whether this build reads what a connection carries.
    pub(crate) const OBSERVES: bool = false;

    pub(crate) fn mark(&self, _reason: CloseReason) {}

    pub(crate) fn bytes_written(&self, _bytes: usize) {}

    pub(crate) fn bytes_read(&self, _bytes: usize) {}

    pub(crate) fn eof(&self) {}

    pub(crate) fn classify(&self, _ended: &Result<(), hyper::Error>) -> CloseReason {
        CloseReason::Other
    }
}

/// A connection's place in the registry.
pub(crate) struct ConnGuard;

impl ConnGuard {
    pub(crate) fn end(self, _reason: CloseReason) {}
}

/// Reads the HTTP/2 frames of what a connection receives.
#[derive(Debug, Default)]
pub(crate) struct Sniffer;

impl Sniffer {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn feed(&mut self, _data: &[u8], _conn: &ConnCounters) {}
}
