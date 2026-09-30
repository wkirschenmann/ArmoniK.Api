use std::sync::Arc;

use armonik_transport::grpc::{CallStartOptions, Metadata};

use super::{actor, CallServices};
use crate::abi::{ak_handle, ak_status};
use crate::channel::AkChannel;
use crate::host::HostPtr;
use crate::tables;

/// The channel's count, given back unless the call that took it is started.
///
/// Every exit from `start_on` between the join and the call being live has to give it back, an
/// unwind included: ak_call_start turns a panic into AK_STATUS_INTERNAL, and a count left
/// standing is a channel that reaches CLOSING and never CLOSED.
struct Joined<'a>(Option<&'a Arc<AkChannel>>);

impl Joined<'_> {
    fn kept(mut self) {
        self.0 = None;
    }
}

impl Drop for Joined<'_> {
    fn drop(&mut self) {
        if let Some(channel) = self.0 {
            channel.leave(None);
        }
    }
}

pub(crate) fn start_on(
    channel: &Arc<AkChannel>,
    services: &CallServices<'_>,
    method: &str,
    metadata: Metadata,
    ctx: HostPtr,
) -> Result<ak_handle, ak_status> {
    channel.join()?;
    let joined = Joined(Some(channel));

    let mut options = CallStartOptions::new(method);
    options.metadata = metadata;

    let grpc_call = match channel.grpc.start_call(options) {
        Ok(call) => call,
        Err(error) => return Err(ak_status::from(error)),
    };

    let (send, recv, control) = grpc_call.split();
    let inserted = tables::calls().insert_with(|handle| {
        let (state, commands) = actor::create(
            ctx,
            handle,
            Arc::clone(channel),
            services,
            control,
            channel.max_sends_in_flight,
            channel.delivery_credits,
        );
        (Arc::clone(&state), (state, commands))
    });

    let Some((handle, (state, commands))) = inserted else {
        return Err(ak_status::AK_STATUS_INTERNAL);
    };

    // A release cancels the calls its channel lists, and one that ran since the join took its
    // list without this call - which would then be a live call on a closing channel, which the
    // header says cannot be. Listing it takes the lock the release took, so exactly one of the
    // two cancels it.
    if !channel.enlist(handle) {
        state.cancel();
    }

    // From here the call is the channel's to count, and its terminal is what gives the count
    // back.
    joined.kept();
    actor::start(&state, send, recv, commands, services.spawner);
    Ok(handle)
}
