use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use armonik_transport::grpc::{
    CallControl, CallDriver, CallError, GrpcStatus, GrpcStatusCode, ResponseHead, ResponseSink,
    SendHalf,
};
use bytes::Bytes;
use tokio::sync::{mpsc, oneshot, watch, Semaphore};

use super::lent::lend_payload;
use super::{CallServices, CallState, CallTask, Command, Debt, ReadTurn, Requests};
use crate::abi::{ak_event, ak_event_kind, ak_handle, ak_head_origin, ak_host_debt};
use crate::blob;
use crate::channel::AkChannel;
use crate::host::HostPtr;
use crate::ledger::{Received, Waiter};

pub(super) fn create(
    ctx: HostPtr,
    handle: ak_handle,
    channel: Arc<AkChannel>,
    services: &CallServices<'_>,
    control: CallControl,
    turn: Arc<ReadTurn>,
    (requests, task): (Requests, CallTask),
) -> Arc<CallState> {
    Arc::new(CallState {
        ctx,
        host: Arc::clone(services.host),
        ledger: Arc::clone(services.ledger),
        control,
        requests,
        task: Mutex::new(Some(task)),
        window: Semaphore::new(channel.max_sends_in_flight),
        credits: Semaphore::new(channel.delivery_credits),
        debt: Debt::default(),
        cancelled: AtomicBool::new(false),
        progress: watch::channel(0).0,
        over: watch::channel(false).0,
        sending: AtomicU32::new(0),
        waiter: Arc::new(Waiter::default()),
        turn,
        channel,
        handle,
    })
}

/// A stream's commands to its writer, and what its task is made of.
pub(super) fn streaming(
    driver: CallDriver,
    send: SendHalf,
    channel: &AkChannel,
) -> (Requests, CallTask) {
    let (tx, rx) = mpsc::channel(channel.max_sends_in_flight + 1);
    (
        Requests::Stream(tx),
        CallTask {
            driver,
            writing: Some((send, rx)),
        },
    )
}

/// The call's one task: the transport's driver, which delivers the response itself, and the
/// writer - or, on a call that sends one request, what stands for it: the budget's wake-ups.
/// One spawn wakes the channel's thread once for both, and they share its task cell.
pub(super) fn spawn(state: &Arc<CallState>, task: CallTask) {
    let (writer_done, writer_is_done) = oneshot::channel();
    let state = Arc::clone(state);
    let spawner = state.channel.spawner.clone();
    let CallTask { driver, writing } = task;

    // The futures are made inside, where they are polled, so the task is laid out with each once.
    // The writer after the driver on every poll, so it sees at once that the driver has ended the
    // call.
    match writing {
        Some((send, commands)) => spawner.spawn(async move {
            tokio::join!(
                biased;
                driver.drive(Delivering::new(Arc::clone(&state), writer_is_done)),
                writer(state, send, commands, writer_done),
            );
        }),
        None => spawner.spawn(async move {
            tokio::join!(
                biased;
                driver.drive(Delivering::new(Arc::clone(&state), writer_is_done)),
                budget_wakes(state, writer_done),
            );
        }),
    };
}

/// What a call with no writer raises in its place: a refused lend's wake-up, until the call is over.
async fn budget_wakes(state: Arc<CallState>, done: oneshot::Sender<()>) {
    let mut over = state.over.subscribe();
    loop {
        tokio::select! {
            biased;
            _ = over.wait_for(|over| *over) => break,
            () = state.waiter.notified() => {
                if state.waiter.take_owed() && state.accepts_work() {
                    state.in_callback(|| {
                        state
                            .host
                            .signal(state.ctx, ak_event_kind::AK_EVENT_BUDGET_WAKE)
                    });
                }
            }
        }
    }
    let _ = done.send(());
}

async fn writer(
    state: Arc<CallState>,
    send: SendHalf,
    commands: mpsc::Receiver<Command>,
    done: oneshot::Sender<()>,
) {
    // Guarded for the acquittal below rather than for the loop's sake: the end waits on it before
    // the terminal, and a panic that skipped it would leave that wait to the oneshot's drop - the
    // same outcome by accident instead of on purpose. What is not recovered is what
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
                // Raised here, by the writer the end waits for before the terminal, so the
                // terminal stays the call's last callback.
                () = state.waiter.notified() => {
                    if state.waiter.take_owed() && state.accepts_work() {
                        state.in_callback(|| {
                            state
                                .host
                                .signal(state.ctx, ak_event_kind::AK_EVENT_BUDGET_WAKE)
                        });
                    }
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
                // Given back inside the callback, so a host that lends again on WRITE_DONE finds
                // the room the message it just sent freed rather than a refusal it cannot explain.
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

/// The call's response, as the driver reads it: each event is staged once it is in hand, which is
/// its delivery step, and what is staged goes to the host in one callback when the driver has
/// nothing more ready, when the delivery window is spent, or with the terminal.
struct Delivering {
    state: Arc<CallState>,
    staged: Staged,
    writer_is_done: oneshot::Receiver<()>,
}

/// Events staged for the host.
struct Staged(Vec<ak_event>);

// SAFETY: a staged event's pointers are into a payload this library owns, which is `Send`, and
// nothing reads through them until the callback hands them to the host.
unsafe impl Send for Staged {}

impl Delivering {
    fn new(state: Arc<CallState>, writer_is_done: oneshot::Receiver<()>) -> Self {
        // A unary answer's head, message and status; a larger batch grows it.
        let staged = Staged(Vec::with_capacity(3));
        Self {
            state,
            staged,
            writer_is_done,
        }
    }

    /// Takes a credit for the event and stages it. With none free, what is staged goes first: the
    /// host gives credits back by consuming, and it cannot consume what it has not been given.
    async fn stage(
        &mut self,
        kind: ak_event_kind,
        data: Bytes,
        code: i32,
        charge: Option<Received>,
    ) -> Result<(), GrpcStatus> {
        // Forgotten like the send window's: the credit is spent until the host consumes the
        // payload, and `ak_event_consumed` is what gives it back.
        if !self.took_credit() {
            self.flush();
            let state = &self.state;
            let permit = tokio::select! {
                biased;
                permit = state.credits.acquire() => permit,
                () = wait_for_cancel(state) => return Err(GrpcStatus::cancelled()),
            };
            match permit {
                Ok(permit) => permit.forget(),
                Err(_) => return Err(GrpcStatus::cancelled()),
            }
        }
        self.push(kind, lend_payload(&self.state, data, true, charge), code);
        Ok(())
    }

    fn took_credit(&self) -> bool {
        self.state
            .credits
            .try_acquire()
            .map(|permit| permit.forget())
            .is_ok()
    }

    fn push(&mut self, kind: ak_event_kind, payload: crate::abi::ak_bytes, code: i32) {
        self.staged.0.push(ak_event {
            kind,
            payload,
            status_code: code,
            host_debt: ak_host_debt::AK_HOST_NOTHING_TO_RETURN,
        });
    }

    fn head_event(head: &ResponseHead) -> (Bytes, i32) {
        (
            Bytes::from(blob::encode_metadata(&head.metadata)),
            ak_head_origin::from(head.origin) as i32,
        )
    }
}

impl ResponseSink for Delivering {
    async fn head(&mut self, head: ResponseHead) -> Result<(), GrpcStatus> {
        let (data, origin) = Self::head_event(&head);
        self.stage(ak_event_kind::AK_EVENT_INITIAL_METADATA, data, origin, None)
            .await
    }

    async fn message(&mut self, data: Bytes) -> Result<(), GrpcStatus> {
        if self.state.cancelled.load(Ordering::Acquire) {
            return Err(GrpcStatus::cancelled());
        }
        // The engine waits on the call's turn before it reads the message off the stream; the
        // message is charged here, once it arrives decoded.
        let Some(charge) = self.state.ledger.hold_received(data.len()) else {
            // Freed at once, and the peer told: past the second threshold the process would run
            // out of memory before the host gave anything back.
            self.state.control.cancel();
            return Err(GrpcStatus::new(
                GrpcStatusCode::ResourceExhausted,
                "the runtime's ceiling on received messages is reached",
            ));
        };
        self.stage(ak_event_kind::AK_EVENT_MESSAGE, data, 0, Some(charge))
            .await?;
        // Delivered once staged: the next read may come before the host has this one.
        self.state.turn.delivered();
        Ok(())
    }

    fn flush(&mut self) {
        if self.staged.0.is_empty() {
            return;
        }
        let Self { state, staged, .. } = self;
        state.in_callback(|| state.host.deliver(state.ctx, &staged.0));
        staged.0.clear();
    }

    async fn end(mut self, status: GrpcStatus, head: Option<ResponseHead>) {
        // A head never given has every credit free to take: nothing before it took one.
        if let Some(head) = head {
            if self.took_credit() {
                let (data, origin) = Self::head_event(&head);
                let payload = lend_payload(&self.state, data, true, None);
                self.push(ak_event_kind::AK_EVENT_INITIAL_METADATA, payload, origin);
            }
        }

        // The status waits for every acquittal owed: the writer, polled after the driver, sees at
        // once that the call is over, acquits what it holds and ends.
        self.state.over.send_replace(true);
        let _ = (&mut self.writer_is_done).await;
        // The call is over, so its send waits on nothing more.
        self.state.ledger.stop_waiting(&self.state.waiter);

        let payload = lend_payload(
            &self.state,
            Bytes::from(blob::status_payload(
                &status.message,
                &status.trailing_metadata,
            )),
            // The terminal was never charged a delivery credit, so consuming it returns none.
            false,
            None,
        );
        self.push(ak_event_kind::AK_EVENT_STATUS, payload, status.code as i32);

        let Self { state, staged, .. } = &mut self;
        state.in_callback(|| {
            state.host.deliver(state.ctx, &staged.0);
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
        staged.0.clear();

        reclaim(state).await;
    }
}

async fn reclaim(state: &Arc<CallState>) {
    let mut progress = state.progress.subscribe();
    let _ = progress.wait_for(|_| state.debt.settled()).await;
    crate::lifecycle::call_settled(state.handle);
}

async fn wait_for_cancel(state: &CallState) {
    let mut progress = state.progress.subscribe();
    let _ = progress
        .wait_for(|_| state.cancelled.load(Ordering::Acquire))
        .await;
}
