use std::collections::HashSet;
use std::hash::BuildHasherDefault;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use armonik_transport::grpc::GrpcChannel;

use crate::abi::{ak_channel_state, ak_error_kind, ak_handle, ak_status};
use crate::config;
use crate::held::Held;
use crate::refusal::Refusal;
use crate::registry::Spread;
use crate::tables;

pub(crate) struct AkChannel {
    pub(crate) grpc: GrpcChannel,
    pub(crate) runtime: ak_handle,
    pub(crate) delivery_credits: usize,
    pub(crate) max_sends_in_flight: usize,
    /// Its own handle, which it takes out of the table once it is released and closed.
    handle: ak_handle,
    members: Mutex<Members>,
}

/// The channel's phase and the calls a close has to cancel, behind the one lock a start, an end
/// and a close all take.
struct Members {
    phase: Phase,
    /// The calls a close cancels: joined, in the calls table, and short of their terminal. A call
    /// is counted from its join and listed only once it is in the table, where a close looks it
    /// up.
    listed: HashSet<ak_handle, BuildHasherDefault<Spread>>,
    /// The host has released the channel, so its handle is the table's to reclaim once it is
    /// closed. A channel the runtime's shutdown closed is still the host's to name.
    released: bool,
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
/// does not apply, which leaves the interleavings to the lock `Members` is behind.
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

impl Members {
    fn open() -> Self {
        Self {
            phase: Phase {
                state: ak_channel_state::AK_CHANNEL_OPEN,
                calls: 0,
            },
            listed: HashSet::default(),
            released: false,
        }
    }

    /// Read and written together, because every transition reads both halves of the phase: a
    /// channel that began closing between the read and the write would otherwise take a call it
    /// has already refused.
    fn advance(&mut self, step: fn(Phase) -> Option<Phase>) -> bool {
        match step(self.phase) {
            Some(next) => {
                self.phase = next;
                true
            }
            None => false,
        }
    }

    fn enlist(&mut self, call: ak_handle) -> bool {
        let open = self.phase.state == ak_channel_state::AK_CHANNEL_OPEN;
        if open {
            self.listed.insert(call);
        }
        open
    }

    fn leave(&mut self, call: Option<ak_handle>) -> bool {
        if let Some(call) = call {
            self.listed.remove(&call);
        }
        self.advance(Phase::left)
    }

    fn close(&mut self) -> Option<Vec<ak_handle>> {
        self.advance(Phase::closing)
            .then(|| self.listed.iter().copied().collect())
    }

    fn release(&mut self) -> Option<Vec<ak_handle>> {
        self.released = true;
        self.close()
    }

    fn reclaimable(&self) -> bool {
        self.released && self.phase.state == ak_channel_state::AK_CHANNEL_CLOSED
    }
}

impl AkChannel {
    fn members(&self) -> Held<MutexGuard<'_, Members>> {
        Held::new(self.members.lock().unwrap_or_else(PoisonError::into_inner))
    }

    pub(crate) fn state(&self) -> ak_channel_state {
        self.members().phase.state
    }

    pub(crate) fn join(&self) -> Result<(), ak_status> {
        if self.members().advance(Phase::joined) {
            return Ok(());
        }
        Err(ak_status::AK_STATUS_INVALID_STATE)
    }

    /// Lists a joined call once it is in the calls table, where a close looks it up. False when a
    /// close has begun since the join: that close took its list without this call, which is then
    /// the one to cancel itself.
    pub(crate) fn enlist(&self, call: ak_handle) -> bool {
        self.members().enlist(call)
    }

    /// A joined call going, by its handle once it has one: a terminal names it, a start that
    /// failed before the call was in the table does not.
    pub(crate) fn leave(&self, call: Option<ak_handle>) {
        // Refusing rather than wrapping: one join is one leave, so the count cannot already be
        // zero, and a wrong count that wrapped would be a channel that never closes again.
        let counted = self.members().leave(call);
        debug_assert!(counted, "a call left a channel it had not joined");
        self.finish_closing();
    }

    /// OPEN -> CLOSING, whatever the count, and the calls to cancel; None when it was not OPEN.
    ///
    /// Through CLOSING even with nothing to drain, because that is what the header describes and
    /// what the model has as two steps. Nothing is lost by it: with no call on the channel, the
    /// close that started it finishes it before returning.
    pub(crate) fn start_closing(&self) -> Option<Vec<ak_handle>> {
        self.members().close()
    }

    /// `start_closing`, by the host: the channel is reclaimed once it is closed.
    pub(crate) fn release(&self) -> Option<Vec<ak_handle>> {
        self.members().release()
    }

    /// CLOSING -> CLOSED, once the last call has left, and the handle reclaimed if the host has
    /// released the channel.
    ///
    /// Called by whoever made that true - the call that left, or the release itself when there
    /// was no call to wait for, or when a shutdown had already closed the channel.
    pub(crate) fn finish_closing(&self) {
        let reclaimable = {
            let mut members = self.members();
            members.advance(Phase::closed);
            members.reclaimable()
        };
        // Outside this channel's lock, because the table's is another. A second remove finds
        // nothing, so the call that left and the release may both get here.
        if reclaimable {
            tables::channels().remove(self.handle);
        }
    }
}

pub(crate) fn create(
    runtime: ak_handle,
    spawner: &tokio::runtime::Handle,
    endpoint: &[u8],
    json: &[u8],
) -> Result<ak_handle, Refusal> {
    // The endpoint is not echoed: a URI may carry credentials in its userinfo.
    let endpoint = std::str::from_utf8(endpoint)
        .map_err(|_| ENDPOINT_NOT_UTF8)?
        .parse()
        .map_err(|_| ENDPOINT_NOT_A_URI)?;
    let settings = config::parse(json).map_err(Refusal::config)?;
    let delivery_credits = settings.delivery_credits();
    let max_sends_in_flight = settings.max_sends_in_flight();
    let connect_eagerly = settings.connect_eagerly();

    let grpc = GrpcChannel::new(settings.into_channel_config(endpoint), spawner.clone())
        .map_err(Refusal::channel)?;
    let dialled = grpc.clone();

    let handle = tables::channels()
        .insert_with(|handle| {
            let channel = Arc::new(AkChannel {
                grpc,
                runtime,
                delivery_credits,
                max_sends_in_flight,
                handle,
                members: Mutex::new(Members::open()),
            });
            (channel, ())
        })
        .map(|(handle, ())| handle)
        .ok_or(CHANNELS_SPENT)?;

    // Spawned once the channel is the host's, so a refused creation dials nothing. Its failure is
    // the first call's to report: nothing caches a failed dial, so that call dials again, or
    // joins this dial while it runs, and meets the same answer.
    if connect_eagerly {
        spawner.spawn(async move {
            let _ = dialled.connect().await;
        });
    }
    Ok(handle)
}

const ENDPOINT_NOT_UTF8: Refusal = Refusal::fixed(
    ak_status::AK_STATUS_INVALID_ARG,
    ak_error_kind::AK_ERROR_CONFIG,
    "the endpoint is not UTF-8",
);
const ENDPOINT_NOT_A_URI: Refusal = Refusal::fixed(
    ak_status::AK_STATUS_INVALID_ARG,
    ak_error_kind::AK_ERROR_CONFIG,
    "the endpoint is not a URI",
);
const CHANNELS_SPENT: Refusal = Refusal::fixed(
    ak_status::AK_STATUS_INTERNAL,
    ak_error_kind::AK_ERROR_NONE,
    "every channel handle this library can hand out is spent",
);

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
    fn a_close_cancels_the_listed_calls_and_not_one_still_being_entered() {
        let mut members = Members::open();
        assert!(members.advance(Phase::joined));
        assert!(members.enlist(1));
        assert!(members.advance(Phase::joined));

        assert_eq!(members.close(), Some(vec![1]));
    }

    #[test]
    fn a_call_listed_after_the_close_began_is_refused_and_left_off_the_list() {
        let mut members = Members::open();
        assert!(members.advance(Phase::joined));
        assert_eq!(members.close(), Some(vec![]));

        assert!(!members.enlist(1), "the call has to cancel itself");
        assert!(members.listed.is_empty());
    }

    #[test]
    fn a_call_that_leaves_is_off_the_list() {
        let mut members = Members::open();
        assert!(members.advance(Phase::joined));
        assert!(members.enlist(1));
        assert!(members.leave(Some(1)));

        assert_eq!(members.close(), Some(vec![]));
        assert_eq!(members.close(), None, "and a close takes its list once");
    }

    #[test]
    fn the_close_finishes_only_once_the_last_call_has_left() {
        assert_eq!(step((CLOSING, 0), Phase::closed), Some((CLOSED, 0)));
        assert_eq!(step((CLOSING, 1), Phase::closed), None);
        assert_eq!(
            step((OPEN, 0), Phase::closed),
            None,
            "a channel nobody released"
        );
        assert_eq!(
            step((CLOSED, 0), Phase::closed),
            None,
            "and it finishes once"
        );
    }

    #[test]
    fn a_released_channel_is_reclaimable_once_its_last_call_has_left() {
        let mut members = Members::open();
        assert!(members.advance(Phase::joined));
        assert_eq!(members.release(), Some(vec![]));
        assert!(!members.reclaimable(), "a call is still on it");

        assert!(members.leave(None));
        assert!(members.advance(Phase::closed));
        assert!(members.reclaimable());
    }

    #[test]
    fn a_channel_the_shutdown_closed_stays_the_hosts_until_it_releases_it() {
        let mut members = Members::open();
        assert_eq!(members.close(), Some(vec![]));
        assert!(members.advance(Phase::closed));
        assert!(!members.reclaimable(), "the host still names it");

        assert_eq!(members.release(), None, "there is nothing left to cancel");
        assert!(members.reclaimable());
    }
}
