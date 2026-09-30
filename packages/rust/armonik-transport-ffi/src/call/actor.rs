use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use armonik_transport::grpc::{
    CallControl, CallError, GrpcStatus, GrpcStatusCode, Metadata, RecvHalf, RecvResult, SendHalf,
};
use bytes::Bytes;
use tokio::sync::{mpsc, oneshot, watch, Semaphore};

use super::lent::lend_payload;
use super::{CallServices, CallState, Command, Debt};
use crate::abi::{ak_event_kind, ak_handle, ak_head_origin};
use crate::blob;
use crate::channel::AkChannel;
use crate::host::HostPtr;

pub(super) fn create(
    ctx: HostPtr,
    handle: ak_handle,
    channel: Arc<AkChannel>,
    services: &CallServices<'_>,
    control: CallControl,
    max_sends_in_flight: usize,
    delivery_credits: usize,
) -> (Arc<CallState>, mpsc::Receiver<Command>) {
    let (tx, rx) = mpsc::channel(max_sends_in_flight + 1);

    let state = Arc::new(CallState {
        ctx,
        host: Arc::clone(services.host),
        ledger: Arc::clone(services.ledger),
        control,
        commands: tx,
        window: Semaphore::new(max_sends_in_flight),
        credits: Semaphore::new(delivery_credits),
        debt: Debt::default(),
        cancelled: AtomicBool::new(false),
        progress: watch::channel(0).0,
        over: watch::channel(false).0,
        sending: AtomicU32::new(0),
        channel,
        handle,
    });
    (state, rx)
}

pub(super) fn start(
    state: &Arc<CallState>,
    send: SendHalf,
    recv: RecvHalf,
    commands: mpsc::Receiver<Command>,
    spawner: &tokio::runtime::Handle,
) {
    let (writer_done, writer_is_done) = oneshot::channel();

    spawner.spawn(writer(Arc::clone(state), send, commands, writer_done));
    spawner.spawn(reader(Arc::clone(state), recv, writer_is_done));
}

async fn writer(
    state: Arc<CallState>,
    send: SendHalf,
    commands: mpsc::Receiver<Command>,
    done: oneshot::Sender<()>,
) {
    // Guarded for the acquittal below rather than for the loop's sake: the reader waits on it
    // before the terminal, and a panic that skipped it would leave that wait to the oneshot's
    // drop - the same outcome by accident instead of on purpose. What is not recovered is what
    // the panicking send owed: its window permit and its charge against the ledger, so a runtime
    // that meets this may not empty its ledger again.
    let _ = crate::guarded(write_until_closed(&state, send, commands)).await;

    let _ = done.send(());
}

async fn write_until_closed(
    state: &Arc<CallState>,
    send: SendHalf,
    mut commands: mpsc::Receiver<Command>,
) {
    let mut send = Some(send);
    let mut over = state.over.subscribe();
    let mut draining = false;

    loop {
        let command = if draining {
            commands.recv().await
        } else {
            tokio::select! {
                biased;
                _ = over.wait_for(|over| *over) => {
                    commands.close();
                    draining = true;
                    continue;
                }
                command = commands.recv() => command,
            }
        };

        let Some(command) = command else { break };
        match command {
            Command::Send(bytes) => {
                let charged = bytes.len();
                if let Some(half) = send.as_mut() {
                    // `Ended` is the one refusal a message that came through the ABI can meet, and
                    // it changes nothing below: WRITE_DONE settles an accepted send and says
                    // nothing about the network, so a message abandoned because the call ended is
                    // acquitted like one written and the terminal is what reports the end.
                    // Withholding it instead would break the header's "exactly once per accepted
                    // send" and hang a host waiting for its acquittal.
                    //
                    // The other refusal, a length no four-byte prefix can carry, is refused at the
                    // lend: `LARGEST_LENDABLE` caps the ceiling itself, so it cannot reach here to
                    // be lost behind an acquittal. Asserted rather than argued, because the day
                    // that stops being true is the day a message goes missing in silence.
                    let sent = half.send_message(bytes).await;
                    debug_assert!(
                        matches!(sent, Ok(()) | Err(CallError::Ended)),
                        "a send accepted at the lend was refused by the transport: {sent:?}"
                    );
                }
                // Given back inside the callback, so a host that lends again on WRITE_DONE finds the
                // room the message it just sent freed rather than a refusal it cannot explain.
                state.in_callback(|| {
                    state.window.add_permits(1);
                    state.ledger.release_bytes(charged);
                    state
                        .host
                        .signal(state.ctx, ak_event_kind::AK_EVENT_WRITE_DONE)
                });
            }
            Command::EndSend => {
                if let Some(half) = send.take() {
                    // Nothing to report: the half-close is this half being dropped, which ends the
                    // request body, so `end_send` has no wire step a refusal could name.
                    let _ = half.end_send().await;
                }
            }
        }
    }
}

async fn reader(state: Arc<CallState>, recv: RecvHalf, writer_is_done: oneshot::Receiver<()>) {
    let status = crate::guarded(read_until_end(&state, recv))
        .await
        .unwrap_or_else(|| {
            // The read side panicked, which is a bug here and not the peer's doing. The terminal
            // goes out all the same: a host waiting for one it will never get is the hang
            // requirement 14.8 exists to forbid.
            GrpcStatus::new(
                GrpcStatusCode::Internal,
                "the transport failed while reading the response",
            )
        });

    state.over.send_replace(true);
    let _ = writer_is_done.await;

    {
        let payload = lend_payload(
            &state,
            Bytes::from(blob::status_payload(
                &status.message,
                &status.trailing_metadata,
            )),
            // The terminal was never charged a delivery credit, so consuming it returns none.
            false,
        );
        state.in_callback(|| {
            state.host.deliver(
                state.ctx,
                ak_event_kind::AK_EVENT_STATUS,
                payload,
                status.code as i32,
            );
            // Sequentially consistent, not Release: `lend` claims a buffer and then reads this,
            // `settled` reads this and then the claim, and the argument that one of the two sees
            // the other's write holds only if every one of those is in the single total order.
            // A Release store does not join it.
            state.debt.terminal.store(true, Ordering::SeqCst);

            // Inside the callback, so the call has left its channel before it can report
            // itself quiet - which is what `begin_shutdown` waits on for every call before the
            // runtime is quiescent.
            state.channel.leave(Some(state.handle));
        });
    }

    reclaim(&state).await;
}

/// Everything the read side does before the terminal: the head, then a message at a time.
async fn read_until_end(state: &Arc<CallState>, mut recv: RecvHalf) -> GrpcStatus {
    let (head, origin) = match recv.recv_head().await {
        Ok(head) => (
            blob::encode_metadata(&head.metadata),
            ak_head_origin::from(head.origin),
        ),
        // The driver was dropped before it handed over a head, so none reached the call.
        Err(_) => (
            blob::encode_metadata(&Metadata::new()),
            ak_head_origin::AK_HEAD_NO_RESPONSE,
        ),
    };
    let delivered_head = deliver(
        state,
        ak_event_kind::AK_EVENT_INITIAL_METADATA,
        Bytes::from(head),
        origin as i32,
    )
    .await;

    if !delivered_head {
        return GrpcStatus::cancelled();
    }

    loop {
        if state.cancelled.load(Ordering::Acquire) {
            break GrpcStatus::cancelled();
        }

        match recv.next_message().await {
            Ok(RecvResult::Message(message)) => {
                if !deliver(state, ak_event_kind::AK_EVENT_MESSAGE, message.data, 0).await {
                    break GrpcStatus::cancelled();
                }
            }
            Ok(RecvResult::End(status)) => break status,
            Err(_) => break GrpcStatus::cancelled(),
        }
    }
}

async fn reclaim(state: &Arc<CallState>) {
    let mut progress = state.progress.subscribe();
    let _ = progress.wait_for(|_| state.debt.settled()).await;
    crate::lifecycle::call_settled(state.handle);
}

async fn deliver(state: &Arc<CallState>, kind: ak_event_kind, data: Bytes, code: i32) -> bool {
    let permit = tokio::select! {
        biased;
        permit = state.credits.acquire() => permit,
        () = wait_for_cancel(state) => return false,
    };
    // Forgotten like the send window's: the credit is spent until the host consumes the payload,
    // and `ak_event_consumed` is what gives it back.
    match permit {
        Ok(permit) => permit.forget(),
        Err(_) => return false,
    }

    let payload = lend_payload(state, data, true);
    state.in_callback(|| state.host.deliver(state.ctx, kind, payload, code));
    true
}

async fn wait_for_cancel(state: &CallState) {
    let mut progress = state.progress.subscribe();
    let _ = progress
        .wait_for(|_| state.cancelled.load(Ordering::Acquire))
        .await;
}
