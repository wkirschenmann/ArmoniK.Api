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
    pub(crate) handle: ak_handle,
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

impl AkChannel {
    pub(crate) fn state(&self) -> ak_channel_state {
        parts(self.state.load(Ordering::Acquire)).0
    }

    /// Compare-and-swap rather than a load and an add, because the state and the count share the
    /// word: a channel that begins closing between the two takes no further call, which is what
    /// lets `start_closing` decide on the count it just read.
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

    pub(crate) fn leave(&self) {
        let mut seen = self.state.load(Ordering::Acquire);
        loop {
            let (state, calls) = parts(seen);
            let left = calls - 1;
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

    pub(crate) fn start_closing(&self) -> bool {
        let mut seen = self.state.load(Ordering::Acquire);
        loop {
            let (state, calls) = parts(seen);
            if state != ak_channel_state::AK_CHANNEL_OPEN {
                return false;
            }
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

pub(crate) fn create(
    runtime: ak_handle,
    spawner: &tokio::runtime::Handle,
    json: &[u8],
) -> Result<ak_handle, ak_status> {
    let (settings, endpoint) = config::parse(json).ok_or(ak_status::AK_STATUS_INVALID_ARG)?;
    let delivery_credits = settings.delivery_credits();
    let max_sends_in_flight = settings.max_sends_in_flight();

    let grpc = GrpcChannel::new(settings.into_channel_config(endpoint), spawner.clone())
        .map_err(|_| ak_status::AK_STATUS_INVALID_ARG)?;

    tables::channels()
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
        .map(|(handle, ())| handle)
        .ok_or(ak_status::AK_STATUS_INTERNAL)
}
