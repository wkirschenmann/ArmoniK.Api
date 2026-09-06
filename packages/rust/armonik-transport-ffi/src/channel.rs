use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use armonik_transport::grpc::GrpcChannel;

use crate::abi::{ak_channel_state, ak_handle, ak_status};
use crate::config;
use crate::tables;

const STATE_SHIFT: u32 = 32;

pub(crate) struct AkChannel {
    pub(crate) grpc: GrpcChannel,
    pub(crate) runtime: ak_handle,
    pub(crate) delivery_credits: usize,
    pub(crate) max_sends_in_flight: usize,
    state: AtomicU64,
}

fn parts(word: u64) -> (ak_channel_state, u32) {
    let state = ak_channel_state::from_repr((word >> STATE_SHIFT) as i32)
        .unwrap_or(ak_channel_state::AK_CHANNEL_CLOSED);
    (state, word as u32)
}

fn word(state: ak_channel_state, calls: u32) -> u64 {
    ((state as i32 as u64) << STATE_SHIFT) | u64::from(calls)
}

/// The four transitions, as a table over the word. Each answers with the word it moves to, or
/// nothing when it does not apply, which is what leaves the interleavings to `advance` alone.
fn joined(seen: u64) -> Option<u64> {
    let (state, calls) = parts(seen);
    (state == ak_channel_state::AK_CHANNEL_OPEN).then(|| word(state, calls + 1))
}

fn left(seen: u64) -> Option<u64> {
    let (state, calls) = parts(seen);
    (calls > 0).then(|| word(state, calls - 1))
}

fn closing(seen: u64) -> Option<u64> {
    let (state, calls) = parts(seen);
    (state == ak_channel_state::AK_CHANNEL_OPEN)
        .then(|| word(ak_channel_state::AK_CHANNEL_CLOSING, calls))
}

fn closed(seen: u64) -> Option<u64> {
    let (state, calls) = parts(seen);
    (state == ak_channel_state::AK_CHANNEL_CLOSING && calls == 0)
        .then(|| word(ak_channel_state::AK_CHANNEL_CLOSED, 0))
}

impl AkChannel {
    pub(crate) fn state(&self) -> ak_channel_state {
        parts(self.state.load(Ordering::Acquire)).0
    }

    /// Compare-and-swap rather than a load and a store, because the state and the count share
    /// the word: a channel that begins closing between the two would otherwise take a call it
    /// has already refused.
    fn advance(&self, step: fn(u64) -> Option<u64>) -> bool {
        let mut seen = self.state.load(Ordering::Acquire);
        loop {
            let Some(next) = step(seen) else {
                return false;
            };
            match self
                .state
                .compare_exchange_weak(seen, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return true,
                Err(current) => seen = current,
            }
        }
    }

    pub(crate) fn join(&self) -> Result<(), ak_status> {
        if self.advance(joined) {
            return Ok(());
        }
        Err(ak_status::AK_STATUS_INVALID_STATE)
    }

    pub(crate) fn leave(&self) {
        // Refusing rather than wrapping: one join is one leave, so the count cannot already be
        // zero, and a wrong count that wrapped would be a channel that never closes again.
        let counted = self.advance(left);
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
        self.advance(closing)
    }

    /// CLOSING -> CLOSED, once the last call has left.
    ///
    /// Called by whoever made that true - the call that left, or the release itself when there
    /// was no call to wait for.
    pub(crate) fn finish_closing(&self) {
        self.advance(closed);
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
            state: AtomicU64::new(word(ak_channel_state::AK_CHANNEL_OPEN, 0)),
        }))
        .ok_or(ak_status::AK_STATUS_INTERNAL)
}

#[cfg(test)]
mod tests {
    use super::*;

    use ak_channel_state::{
        AK_CHANNEL_CLOSED as CLOSED, AK_CHANNEL_CLOSING as CLOSING, AK_CHANNEL_NONE as NONE,
        AK_CHANNEL_OPEN as OPEN,
    };

    fn step(
        from: (ak_channel_state, u32),
        by: fn(u64) -> Option<u64>,
    ) -> Option<(ak_channel_state, u32)> {
        by(word(from.0, from.1)).map(parts)
    }

    #[test]
    fn only_an_open_channel_takes_a_call() {
        assert_eq!(step((OPEN, 0), joined), Some((OPEN, 1)));
        assert_eq!(step((OPEN, 7), joined), Some((OPEN, 8)));
        assert_eq!(step((CLOSING, 1), joined), None);
        assert_eq!(step((CLOSED, 0), joined), None);
    }

    #[test]
    fn a_call_leaves_without_moving_the_state() {
        assert_eq!(step((OPEN, 1), left), Some((OPEN, 0)));
        assert_eq!(step((CLOSING, 2), left), Some((CLOSING, 1)));
    }

    #[test]
    fn a_count_at_zero_refuses_to_wrap() {
        // A wrap would be a channel with four billion calls, which nothing ever finishes.
        assert_eq!(step((OPEN, 0), left), None);
        assert_eq!(step((CLOSING, 0), left), None);
    }

    #[test]
    fn closing_goes_through_closing_whatever_the_count() {
        assert_eq!(step((OPEN, 0), closing), Some((CLOSING, 0)));
        assert_eq!(step((OPEN, 3), closing), Some((CLOSING, 3)));
    }

    #[test]
    fn a_channel_already_closing_or_closed_starts_nothing() {
        assert_eq!(step((CLOSING, 2), closing), None);
        assert_eq!(step((CLOSED, 0), closing), None);
    }

    #[test]
    fn the_close_finishes_only_once_the_last_call_has_left() {
        assert_eq!(step((CLOSING, 0), closed), Some((CLOSED, 0)));
        assert_eq!(step((CLOSING, 1), closed), None);
        assert_eq!(step((OPEN, 0), closed), None, "a channel nobody released");
        assert_eq!(step((CLOSED, 0), closed), None, "and it finishes once");
    }

    #[test]
    fn a_word_is_its_state_and_its_count_and_nothing_else() {
        for state in [NONE, OPEN, CLOSING, CLOSED] {
            for calls in [0u32, 1, 0xffff, u32::MAX] {
                assert_eq!(parts(word(state, calls)), (state, calls));
            }
        }
    }

    #[test]
    fn a_state_the_library_never_wrote_reads_as_closed() {
        // The word is only ever written by the functions above, so this is unreachable; it
        // answers CLOSED rather than panicking because a channel that cannot be understood is
        // one no call may join.
        assert_eq!(parts(0xdead_0000_0000_0002).0, CLOSED);
    }
}
