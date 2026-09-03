//! The sequences that span more than one kind of object.
//!
//! A channel closing has to reach its calls; a call settling has to be able to finish the channel
//! it was the last of. Neither belongs to either module - a call that knew about channels and a
//! channel that knew about calls would be a cycle, and each would be carrying half of a rule it
//! cannot state on its own. They are stated here, once, in the order they must happen.

use crate::abi::{ak_channel_state, ak_handle};
use crate::tables;

/// Closes a channel and cancels its calls, in that order and only once.
///
/// The cancellation is not a courtesy: the channel is closing from this moment and no closing
/// channel may have an active call, so the latch is what makes the drain this library's own
/// business rather than something the host must provoke. A call parked on a delivery credit is
/// the case that needs it - it is not watching the transport, so closing the session alone would
/// never reach it.
pub(crate) fn release_channel(handle: ak_handle) {
    let Some(found) = tables::channels().get(handle) else {
        return;
    };
    if !found.start_closing() {
        return;
    }

    for call in tables::calls().values() {
        if call.belongs_to_channel(handle) {
            call.cancel();
        }
    }
    found.grpc.close();

    // An idle channel is closed the moment it is released; one with calls closes when its last
    // one is reclaimed.
    settle_channel(handle);
}

/// Takes a settled call's handle back, and closes its channel if it was the last one open.
pub(crate) fn call_settled(call: ak_handle, channel: ak_handle) {
    tables::calls().remove(call);
    settle_channel(channel);
}

/// Marks a closing channel closed once no call of it is left.
///
/// Only a closing channel finishes closing: an open one with no calls is idle, not done.
fn settle_channel(handle: ak_handle) {
    let Some(found) = tables::channels().get(handle) else {
        return;
    };
    if found.state() != ak_channel_state::AK_CHANNEL_CLOSING {
        return;
    }
    if !tables::calls()
        .values()
        .into_iter()
        .any(|call| call.belongs_to_channel(handle))
    {
        found.finish_closing();
    }
}
