//! A channel: one HTTP/2 session's worth of calls, and how it closes.
//!
//! What it owns is its own state: the latch from open to closing to closed, and the session
//! behind it. The sequences that reach other objects - cancelling its calls, deciding it has
//! drained - are in `lifecycle`, because a channel that knew about calls would be half of a
//! cycle. Nothing here decides when a runtime stops either.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use armonik_transport::grpc::{GrpcChannel, TokioExecutor};

use crate::abi::{ak_channel_state, ak_handle, ak_status};
use crate::config;
use crate::tables;

/// Where the state sits in the word the channel keeps; the calls sit below it.
const STATE_SHIFT: u32 = 32;

/// A channel, and the runtime handle it was opened on.
pub(crate) struct AkChannel {
    pub(crate) grpc: GrpcChannel,
    /// Which runtime opened it, as a token and not a pointer - the same choice the ABI makes
    /// for every handle, and what keeps this module independent of the runtime's own.
    pub(crate) runtime: ak_handle,
    pub(crate) handle: ak_handle,
    pub(crate) delivery_credits: usize,
    pub(crate) max_sends_in_flight: usize,
    /// The state and the calls that have not reached their terminal, in one word.
    ///
    /// One word because the two are one question. A channel closes when it is closing and has no
    /// active call, and a call may only join a channel that is open - and with the state and the
    /// count apart, neither of those could be decided: a release could read a count of zero and
    /// publish CLOSED while a start incremented it, leaving the host told CLOSED with events of
    /// that call still to come. Joining and closing are compare-and-swaps on this word instead,
    /// so a call cannot join a channel that is closing and a channel cannot close under a call
    /// that just joined.
    state: AtomicU64,
}

/// The state and the active-call count the word holds.
fn parts(word: u64) -> (ak_channel_state, u32) {
    let state = ak_channel_state::from_repr((word >> STATE_SHIFT) as i32)
        .unwrap_or(ak_channel_state::AK_CHANNEL_CLOSED);
    (state, word as u32)
}

fn word(state: ak_channel_state, calls: u32) -> u64 {
    ((state as i32 as u64) << STATE_SHIFT) | u64::from(calls)
}

impl AkChannel {
    /// The state the host sees, which is a read.
    pub(crate) fn state(&self) -> ak_channel_state {
        parts(self.state.load(Ordering::Acquire)).0
    }

    /// Takes a place among this channel's calls, or refuses because the channel is not open.
    ///
    /// The refusal is the point: a closing channel takes no new call, so nothing has to be
    /// cancelled after the fact and no call is ever counted against a channel that has already
    /// told the host it is CLOSED.
    pub(crate) fn join(&self) -> Result<(), ak_status> {
        let mut seen = self.state.load(Ordering::Acquire);
        loop {
            let (state, calls) = parts(seen);
            if state != ak_channel_state::AK_CHANNEL_OPEN {
                return Err(ak_status::AK_STATUS_INVALID_STATE);
            }
            match self.state.compare_exchange_weak(
                seen,
                word(state, calls + 1),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(()),
                Err(current) => seen = current,
            }
        }
    }

    /// Gives that place back, and closes the channel if it was the last one it was waiting for.
    pub(crate) fn leave(&self) {
        let mut seen = self.state.load(Ordering::Acquire);
        loop {
            let (state, calls) = parts(seen);
            let left = calls - 1;
            // The same step: a closing channel with nothing left is closed, and deciding that
            // separately is what would let a call join in between.
            let next = if left == 0 && state == ak_channel_state::AK_CHANNEL_CLOSING {
                word(ak_channel_state::AK_CHANNEL_CLOSED, 0)
            } else {
                word(state, left)
            };
            match self
                .state
                .compare_exchange_weak(seen, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return,
                Err(current) => seen = current,
            }
        }
    }

    /// Latches the channel closing, and closed at once if nothing of it is active.
    ///
    /// Answers false if it was already closing, so a second release cancels nothing twice.
    pub(crate) fn start_closing(&self) -> bool {
        let mut seen = self.state.load(Ordering::Acquire);
        loop {
            let (state, calls) = parts(seen);
            if state != ak_channel_state::AK_CHANNEL_OPEN {
                return false;
            }
            // An idle channel is closed the moment it is released; one with calls closes when
            // the last of them reaches its terminal.
            let next = if calls == 0 {
                word(ak_channel_state::AK_CHANNEL_CLOSED, 0)
            } else {
                word(ak_channel_state::AK_CHANNEL_CLOSING, calls)
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
}

/// Builds a channel of `runtime` from the host's config JSON, and publishes it.
///
/// Performs no I/O, so it fails only on a configuration this library will not accept - which is
/// why a refusal here says nothing about the endpoint being reachable.
pub(crate) fn create(
    runtime: ak_handle,
    spawner: &tokio::runtime::Handle,
    json: &[u8],
) -> Result<ak_handle, ak_status> {
    let (settings, endpoint) = config::parse(json).ok_or(ak_status::AK_STATUS_INVALID_ARG)?;
    let delivery_credits = settings.delivery_credits();
    let max_sends_in_flight = settings.max_sends_in_flight();

    let executor = TokioExecutor::new(spawner.clone());
    let grpc = GrpcChannel::new(settings.into_channel_config(endpoint), executor)
        .map_err(|_| ak_status::AK_STATUS_INVALID_ARG)?;

    Ok(tables::channels()
        .insert_with(|handle| {
            let channel = AkChannel {
                grpc,
                runtime,
                handle,
                delivery_credits,
                max_sends_in_flight,
                state: AtomicU64::new(word(ak_channel_state::AK_CHANNEL_OPEN, 0)),
            };
            (Arc::new(channel), ())
        })
        .0)
}
