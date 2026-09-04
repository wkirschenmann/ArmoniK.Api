//! A channel: one HTTP/2 session's worth of calls, and how it closes.
//!
//! What it owns is its own state: the latch from open to closing to closed, and the session
//! behind it. The sequences that reach other objects - cancelling its calls, deciding it has
//! drained - are in `lifecycle`, because a channel that knew about calls would be half of a
//! cycle. Nothing here decides when a runtime stops either.

use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::Arc;

use armonik_transport::grpc::{GrpcChannel, TokioExecutor};

use crate::abi::{ak_channel_state, ak_handle, ak_status};
use crate::config;
use crate::tables;

/// A channel, and the runtime handle it was opened on.
pub(crate) struct AkChannel {
    pub(crate) grpc: GrpcChannel,
    /// Which runtime opened it, as a token and not a pointer - the same choice the ABI makes
    /// for every handle, and what keeps this module independent of the runtime's own.
    pub(crate) runtime: ak_handle,
    pub(crate) handle: ak_handle,
    pub(crate) delivery_credits: usize,
    pub(crate) max_sends_in_flight: usize,
    /// Open, closing, closed - as an `ak_channel_state` discriminant, so the observer is a read.
    closing: AtomicU8,
    /// Calls of this channel that have not reached their terminal.
    ///
    /// The same question `CallState::active` answers, counted here so a closing channel does not
    /// walk the whole call table once per call that settles. Derived state, and the derivation
    /// is only ever wrong in the safe direction: the count drops just after a call stores its
    /// terminal, so a stale read holds a channel CLOSING a moment longer and never closes one
    /// early.
    active_calls: AtomicU32,
}

impl AkChannel {
    /// A call of this channel has started. Counted before it is published, so the count is never
    /// lower than the table would say.
    pub(crate) fn call_started(&self) {
        self.active_calls.fetch_add(1, Ordering::AcqRel);
    }

    /// A call of this channel has reached its terminal.
    pub(crate) fn call_ended(&self) {
        self.active_calls.fetch_sub(1, Ordering::AcqRel);
    }

    /// Whether any call of this channel is still short of its terminal.
    pub(crate) fn has_active_call(&self) -> bool {
        self.active_calls.load(Ordering::Acquire) > 0
    }

    pub(crate) fn state(&self) -> ak_channel_state {
        // Stored narrow because the latch is one word; read through the module that numbers it.
        ak_channel_state::from_repr(self.closing.load(Ordering::Acquire) as i32)
            .unwrap_or(ak_channel_state::AK_CHANNEL_CLOSED)
    }

    /// Latches the channel to closing. Answers false if it was already, so a second release
    /// cancels nothing twice.
    pub(crate) fn start_closing(&self) -> bool {
        self.closing
            .compare_exchange(
                ak_channel_state::AK_CHANNEL_OPEN as u8,
                ak_channel_state::AK_CHANNEL_CLOSING as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    /// Closed once nothing of it is active. Only a closing channel finishes closing: an open one
    /// with no calls is idle, not done.
    pub(crate) fn finish_closing(&self) {
        let _ = self.closing.compare_exchange(
            ak_channel_state::AK_CHANNEL_CLOSING as u8,
            ak_channel_state::AK_CHANNEL_CLOSED as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
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
                closing: AtomicU8::new(ak_channel_state::AK_CHANNEL_OPEN as u8),
                active_calls: AtomicU32::new(0),
            };
            (Arc::new(channel), ())
        })
        .0)
}
