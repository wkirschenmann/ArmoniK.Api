use std::sync::Arc;

use armonik_transport::grpc::{CallStartOptions, Deadline, Metadata};

use super::{actor, CallServices, CallTask, ReadTurn, Requests};
use crate::abi::{ak_error_kind, ak_handle, ak_status};
use crate::channel::AkChannel;
use crate::host::HostPtr;
use crate::refusal::Refusal;
use crate::tables;

/// What a call declared at its start of the messages each way carries.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Shape {
    pub(crate) one_request: bool,
    pub(crate) one_response: bool,
}

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
    shape: Shape,
    ctx: HostPtr,
) -> Result<ak_handle, Refusal> {
    let Shape {
        one_request,
        one_response,
    } = shape;
    channel.join()?;
    let joined = Joined(Some(channel));

    let turn = Arc::new(ReadTurn::new(Arc::clone(services.ledger)));
    let mut options = CallStartOptions::new(method);
    options.metadata = metadata;
    options.deadline = deadline;
    options.read_gate = Some(Arc::clone(&turn) as _);
    options.one_response = one_response;

    let prepared = if one_request {
        channel
            .grpc
            .prepare_one_request_call(options)
            .map(|(request, control, driver)| {
                let task = CallTask {
                    driver,
                    writing: None,
                };
                (Requests::One(request), control, task)
            })
    } else {
        channel
            .grpc
            .prepare_call(options)
            .map(|(send, control, driver)| {
                let (requests, task) = actor::streaming(driver, send, channel);
                (requests, control, task)
            })
    };
    let (requests, control, task) = prepared.map_err(Refusal::call)?;
    armonik_transport::probe::mark(2);

    let inserted = tables::calls().insert_with(|handle| {
        let state = actor::create(
            ctx,
            handle,
            Arc::clone(channel),
            services,
            control,
            turn,
            (requests, task),
        );
        (Arc::clone(&state), state)
    });

    let Some((handle, state)) = inserted else {
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
    // A call that sends one request is spawned by its commit, or by what needs its task before.
    if !one_request {
        state.spawn_task();
    }
    armonik_transport::probe::mark(3);
    Ok(handle)
}
