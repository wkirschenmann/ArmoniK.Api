use std::sync::{Arc, Mutex, PoisonError};

use armonik_transport::grpc::GrpcChannel;

use crate::abi::{ak_channel_state, ak_handle, ak_status};
use crate::config;
use crate::tables;

pub(crate) struct AkChannel {
    pub(crate) grpc: GrpcChannel,
    pub(crate) runtime: ak_handle,
    pub(crate) delivery_credits: usize,
    pub(crate) max_sends_in_flight: usize,
    phase: Mutex<Phase>,
}

/// What a channel is, and how many calls are on it.
///
/// The two travel together because every transition reads both: a call may join only an open
/// channel, and a close finishes only once the last has left. Behind one lock rather than in one
/// atomic word - the lock is taken on a channel's open and close and on a call's start and end,
/// never on a message, and what it buys is that the pair is a pair rather than a layout.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Phase {
    state: ak_channel_state,
    calls: u32,
}

/// The four transitions, as a table. Each answers with the phase it moves to, or nothing when it
/// does not apply, which is what leaves the interleavings to `advance` alone.
impl Phase {
    fn joined(self) -> Option<Self> {
        (self.state == ak_channel_state::AK_CHANNEL_OPEN).then(|| Self {
            calls: self.calls + 1,
            ..self
        })
    }

    fn left(self) -> Option<Self> {
        (self.calls > 0).then(|| Self {
            calls: self.calls - 1,
            ..self
        })
    }

    fn closing(self) -> Option<Self> {
        (self.state == ak_channel_state::AK_CHANNEL_OPEN).then_some(Self {
            state: ak_channel_state::AK_CHANNEL_CLOSING,
            ..self
        })
    }

    fn closed(self) -> Option<Self> {
        (self.state == ak_channel_state::AK_CHANNEL_CLOSING && self.calls == 0).then_some(Self {
            state: ak_channel_state::AK_CHANNEL_CLOSED,
            calls: 0,
        })
    }
}

impl AkChannel {
    pub(crate) fn state(&self) -> ak_channel_state {
        self.phase
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .state
    }

    /// Read and written under one lock, because every transition reads both halves of the phase:
    /// a channel that began closing between the read and the write would otherwise take a call it
    /// has already refused.
    fn advance(&self, step: fn(Phase) -> Option<Phase>) -> bool {
        let mut phase = self.phase.lock().unwrap_or_else(PoisonError::into_inner);
        match step(*phase) {
            Some(next) => {
                *phase = next;
                true
            }
            None => false,
        }
    }

    pub(crate) fn join(&self) -> Result<(), ak_status> {
        if self.advance(Phase::joined) {
            return Ok(());
        }
        Err(ak_status::AK_STATUS_INVALID_STATE)
    }

    pub(crate) fn leave(&self) {
        // Refusing rather than wrapping: one join is one leave, so the count cannot already be
        // zero, and a wrong count that wrapped would be a channel that never closes again.
        let counted = self.advance(Phase::left);
        debug_assert!(counted, "a call left a channel it had not joined");
        self.finish_closing();
    }

    /// OPEN -> CLOSING, whatever the count.
    ///
    /// Through CLOSING even with nothing to drain, because that is what the header describes and
    /// what the model has as two steps. Nothing is lost by it: `release_channel` finishes the
    /// close before it returns, so a host that reads the status after the call still finds
    /// CLOSED.
    pub(crate) fn start_closing(&self) -> bool {
        self.advance(Phase::closing)
    }

    /// CLOSING -> CLOSED, once the last call has left.
    ///
    /// Called by whoever made that true - the call that left, or the release itself when there
    /// was no call to wait for.
    pub(crate) fn finish_closing(&self) {
        self.advance(Phase::closed);
    }
}

pub(crate) fn create(
    runtime: ak_handle,
    spawner: &tokio::runtime::Handle,
    endpoint: &[u8],
    json: &[u8],
) -> Result<ak_handle, ak_status> {
    let endpoint = std::str::from_utf8(endpoint)
        .ok()
        .and_then(|endpoint| endpoint.parse().ok())
        .ok_or(ak_status::AK_STATUS_INVALID_ARG)?;
    let settings = config::parse(json).ok_or(ak_status::AK_STATUS_INVALID_ARG)?;
    let delivery_credits = settings.delivery_credits();
    let max_sends_in_flight = settings.max_sends_in_flight();

    let grpc = GrpcChannel::new(settings.into_channel_config(endpoint), spawner.clone())
        .map_err(|_| ak_status::AK_STATUS_INVALID_ARG)?;

    tables::channels()
        .insert(Arc::new(AkChannel {
            grpc,
            runtime,
            delivery_credits,
            max_sends_in_flight,
            phase: Mutex::new(Phase {
                state: ak_channel_state::AK_CHANNEL_OPEN,
                calls: 0,
            }),
        }))
        .ok_or(ak_status::AK_STATUS_INTERNAL)
}

#[cfg(test)]
mod tests {
    use super::*;

    use ak_channel_state::{
        AK_CHANNEL_CLOSED as CLOSED, AK_CHANNEL_CLOSING as CLOSING, AK_CHANNEL_OPEN as OPEN,
    };

    fn step(
        from: (ak_channel_state, u32),
        by: fn(Phase) -> Option<Phase>,
    ) -> Option<(ak_channel_state, u32)> {
        by(Phase {
            state: from.0,
            calls: from.1,
        })
        .map(|phase| (phase.state, phase.calls))
    }

    #[test]
    fn only_an_open_channel_takes_a_call() {
        assert_eq!(step((OPEN, 0), Phase::joined), Some((OPEN, 1)));
        assert_eq!(step((OPEN, 7), Phase::joined), Some((OPEN, 8)));
        assert_eq!(step((CLOSING, 1), Phase::joined), None);
        assert_eq!(step((CLOSED, 0), Phase::joined), None);
    }

    #[test]
    fn a_call_leaves_without_moving_the_state() {
        assert_eq!(step((OPEN, 1), Phase::left), Some((OPEN, 0)));
        assert_eq!(step((CLOSING, 2), Phase::left), Some((CLOSING, 1)));
    }

    #[test]
    fn a_count_at_zero_refuses_to_wrap() {
        // A wrap would be a channel with four billion calls, which nothing ever finishes.
        assert_eq!(step((OPEN, 0), Phase::left), None);
        assert_eq!(step((CLOSING, 0), Phase::left), None);
    }

    #[test]
    fn closing_goes_through_closing_whatever_the_count() {
        assert_eq!(step((OPEN, 0), Phase::closing), Some((CLOSING, 0)));
        assert_eq!(step((OPEN, 3), Phase::closing), Some((CLOSING, 3)));
    }

    #[test]
    fn a_channel_already_closing_or_closed_starts_nothing() {
        assert_eq!(step((CLOSING, 2), Phase::closing), None);
        assert_eq!(step((CLOSED, 0), Phase::closing), None);
    }

    #[test]
    fn the_close_finishes_only_once_the_last_call_has_left() {
        assert_eq!(step((CLOSING, 0), Phase::closed), Some((CLOSED, 0)));
        assert_eq!(step((CLOSING, 1), Phase::closed), None);
        assert_eq!(step((OPEN, 0), Phase::closed), None, "a channel nobody released");
        assert_eq!(step((CLOSED, 0), Phase::closed), None, "and it finishes once");
    }
}
