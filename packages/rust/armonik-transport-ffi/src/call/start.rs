use std::sync::Arc;

use armonik_transport::grpc::{CallStartOptions, Deadline, Metadata};

use super::{actor, CallServices, ReadTurn};
use crate::abi::{ak_error_kind, ak_handle, ak_status};
use crate::channel::AkChannel;
use crate::host::HostPtr;
use crate::refusal::Refusal;
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
    deadline: Option<Deadline>,
    ctx: HostPtr,
) -> Result<ak_handle, Refusal> {
    channel.join()?;
    let joined = Joined(Some(channel));

    let turn = Arc::new(ReadTurn::new(Arc::clone(services.ledger)));
    let mut options = CallStartOptions::new(method);
    options.metadata = metadata;
    options.deadline = deadline;
    options.read_gate = Some(Arc::clone(&turn) as _);

    let (grpc_call, driver) = match channel.grpc.prepare_call(options) {
        Ok(prepared) => prepared,
        Err(error) => return Err(Refusal::call(error)),
    };

    let (send, recv, control) = grpc_call.split();
    let inserted = tables::calls().insert_with(|handle| {
        let (state, commands) =
            actor::create(ctx, handle, Arc::clone(channel), services, control, turn);
        (Arc::clone(&state), (state, commands))
    });

    let Some((handle, (state, commands))) = inserted else {
        return Err(Refusal::fixed(
            ak_status::AK_STATUS_INTERNAL,
            ak_error_kind::AK_ERROR_NONE,
            "every call handle this library can hand out is spent",
        ));
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
    actor::start(&state, driver, send, recv, commands, &channel.spawner);
    Ok(handle)
}
