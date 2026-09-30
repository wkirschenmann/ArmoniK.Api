use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use armonik_transport::grpc::{
    CallControl, CallError, CallStartOptions, GrpcStatus, GrpcStatusCode, Metadata, RecvHalf,
    RecvResult, SendHalf,
};
use bytes::Bytes;
use tokio::sync::{mpsc, oneshot, watch, Semaphore};

use crate::abi::{
    ak_buffer, ak_bytes, ak_call_debt, ak_event_kind, ak_handle, ak_head_origin, ak_status,
};
use crate::blob;
use crate::channel::AkChannel;
use crate::host::{Host, HostPtr};
use crate::ledger::Ledger;
use crate::tables;

pub(crate) struct CallServices<'a> {
    pub(crate) host: &'a Arc<Host>,
    pub(crate) ledger: &'a Arc<Ledger>,
    pub(crate) spawner: &'a tokio::runtime::Handle,
}

pub(crate) enum Command {
    Send(Bytes),
    EndSend,
}

#[derive(Default)]
struct Debt {
    payloads: AtomicU32,
    buffers: AtomicU32,
    callbacks: AtomicU32,
    terminal: AtomicBool,
}

impl Debt {
    fn quiet(&self) -> bool {
        // Sequentially consistent, for two handshakes with `lend`: `terminal` against its claim
        // of `buffers`, and, for the shutdown, `terminal` then `Ledger::empty` against its count
        // then its second read of `terminal`. In both, either this side sees the lend or the lend
        // sees the terminal.
        self.terminal.load(Ordering::SeqCst) && self.callbacks.load(Ordering::Acquire) == 0
    }

    fn settled(&self) -> bool {
        self.quiet()
            && self.payloads.load(Ordering::Acquire) == 0
            && self.buffers.load(Ordering::SeqCst) == 0
    }

    fn as_abi(&self) -> ak_call_debt {
        ak_call_debt {
            payloads_owed: self.payloads.load(Ordering::Acquire),
            buffers_lent: self.buffers.load(Ordering::Acquire),
            callbacks_in_flight: self.callbacks.load(Ordering::Acquire),
            terminal_delivered: self.terminal.load(Ordering::Acquire) as i32,
        }
    }
}

pub(crate) struct CallState {
    ctx: HostPtr,
    host: Arc<Host>,
    ledger: Arc<Ledger>,
    control: CallControl,
    commands: mpsc::Sender<Command>,
    window: Semaphore,
    credits: Semaphore,
    debt: Debt,
    cancelled: AtomicBool,
    progress: watch::Sender<u64>,
    over: watch::Sender<bool>,
    ended_sending: AtomicBool,
    /// Held across deciding to queue a command and queueing it.
    ///
    /// The two writers are `ak_call_send_message` and `ak_call_end_send`, and the queue orders
    /// commands by when they are sent, not by when a slot was reserved. Without this, both could
    /// hold a slot, the second could end the sending and queue its command first, and the first
    /// would then queue a send behind it: the writer takes the end, drops the send half, finds no
    /// half for the message, and emits its WRITE_DONE anyway - the host told a message left that
    /// never did. gRPC forbids a host doing the two at once; this makes the answer a refusal
    /// rather than a false acquittal.
    ///
    /// Never held across an await, and taken only by those two downcalls.
    queueing: Mutex<()>,
    handle: ak_handle,
    // The channel itself, not its name: leaving it is not optional, and a name would make it
    // conditional on a lookup whose failure the reader has no answer for.
    channel: Arc<AkChannel>,
}

impl CallState {
    pub(crate) fn debt(&self) -> ak_call_debt {
        self.debt.as_abi()
    }

    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.announce();
        self.control.cancel();
    }

    fn ended_sending(&self) -> bool {
        self.ended_sending.load(Ordering::Acquire)
    }

    fn accepts_work(&self) -> bool {
        self.live() && !self.cancelled.load(Ordering::Acquire)
    }

    fn live(&self) -> bool {
        !*self.over.borrow()
    }

    /// Claims the call's one buffer, then decides whether to fill it.
    ///
    /// The claim comes first because it is what `settled` reads. A lend that took its bytes and
    /// raised the debt afterwards is a lend the reader cannot see: it settles the call, releases
    /// the handle, and the buffer that arrives after it is charged to a ledger nothing will
    /// release - a runtime that never reaches QUIESCENT and an `ak_runtime_destroy` refused for
    /// the life of the process.
    ///
    /// One buffer at a time per call, whatever the send window admits, is the same claim: the
    /// header's SLOT_BUSY wake-up is the call's next WRITE_DONE, and it only says anything
    /// because a host eligible to ask holds nothing.
    pub(crate) fn lend(self: &Arc<Self>, len: usize) -> Result<ak_buffer, ak_status> {
        if self
            .debt
            .buffers
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(ak_status::AK_STATUS_INVALID_STATE);
        }

        // Claimed, then read, against a reader that publishes the terminal and then reads the
        // claim. Both in the sequentially consistent order, so the two cannot pass each other:
        // either the reader sees this claim and does not settle, or this sees the terminal.
        let lent = if self.debt.terminal.load(Ordering::SeqCst) {
            Err(ak_status::AK_STATUS_INVALID_STATE)
        } else {
            self.fill(len)
        };

        if lent.is_err() {
            self.debt.buffers.store(0, Ordering::SeqCst);
            self.moved_on();
        }
        lent
    }

    fn fill(self: &Arc<Self>, len: usize) -> Result<ak_buffer, ak_status> {
        if !self.accepts_work() {
            return Err(ak_status::AK_STATUS_INVALID_STATE);
        }
        // The ceiling covers what the wire and the allocator can carry as well as what the host
        // budgeted: `Ledger` caps one by the other. Refused here rather than at the send, where
        // the engine's refusal is swallowed behind a WRITE_DONE and the host is told a message it
        // never sent has left.
        self.ledger.could_ever_fit(len)?;
        let Ok(slot) = self.window.try_acquire() else {
            return Err(ak_status::AK_STATUS_SLOT_BUSY);
        };
        #[cfg(feature = "test-hooks")]
        crate::hooks::run_before_charge();
        self.ledger.hold_bytes(len)?;

        // Read again once counted: a shutdown that ran after the checks above found nothing
        // owed. The other half is `Debt::quiet` then `Ledger::empty`.
        if self.debt.terminal.load(Ordering::SeqCst) {
            self.ledger.release_bytes(len);
            return Err(ak_status::AK_STATUS_INVALID_STATE);
        }

        // The arena before the permit is spent: a refusal that left the ledger charged and the
        // permit forgotten would be a send window that never opens again.
        let data = match arena(len) {
            Ok(data) => data,
            Err(status) => {
                self.ledger.release_bytes(len);
                return Err(status);
            }
        };

        // Forgotten, not dropped: the permit is spent for as long as the host holds the buffer, and
        // it is the WRITE_DONE that gives it back once the message has left.
        slot.forget();

        let mut lent = Box::new(Lent {
            tag: LENT_TAG,
            call: Arc::clone(self),
            data,
        });
        let ptr = lent.data.as_mut_ptr();
        Ok(ak_buffer {
            ptr,
            len,
            owner: Box::into_raw(lent) as *mut c_void,
        })
    }

    fn took_back(&self, len: usize) {
        self.debt.buffers.fetch_sub(1, Ordering::AcqRel);
        self.ledger.release_bytes(len);
        self.moved_on();
    }

    fn handed_over(&self) {
        self.debt.buffers.fetch_sub(1, Ordering::AcqRel);
        self.moved_on();
    }

    fn payload_returned(&self, returns_credit: bool) {
        if returns_credit {
            self.credits.add_permits(1);
        }
        self.debt.payloads.fetch_sub(1, Ordering::AcqRel);
        self.ledger.release();
        self.moved_on();
    }

    pub(crate) fn commit(&self, lent: Box<Lent>) -> ak_status {
        let _queueing = self.queueing.lock().unwrap_or_else(PoisonError::into_inner);

        if !self.accepts_work() || self.ended_sending() {
            return keep(lent, ak_status::AK_STATUS_INVALID_STATE);
        }

        let Ok(slot) = self.commands.try_reserve() else {
            return keep(lent, ak_status::AK_STATUS_INVALID_STATE);
        };
        self.handed_over();
        slot.send(Command::Send(Bytes::from(lent.data)));
        ak_status::AK_STATUS_OK
    }

    #[allow(clippy::boxed_local)]
    pub(crate) fn give_back(&self, lent: Box<Lent>) {
        self.took_back(lent.data.len());
        self.window.add_permits(1);
    }

    pub(crate) fn end_send(&self) -> ak_status {
        let _queueing = self.queueing.lock().unwrap_or_else(PoisonError::into_inner);

        if !self.live() {
            return ak_status::AK_STATUS_INVALID_STATE;
        }
        let Ok(slot) = self.commands.try_reserve() else {
            return ak_status::AK_STATUS_INVALID_STATE;
        };
        if self.ended_sending.swap(true, Ordering::AcqRel) {
            return ak_status::AK_STATUS_INVALID_STATE;
        }
        slot.send(Command::EndSend);
        ak_status::AK_STATUS_OK
    }

    fn in_callback(&self, emit: impl FnOnce()) {
        self.debt.callbacks.fetch_add(1, Ordering::AcqRel);
        emit();
        self.debt.callbacks.fetch_sub(1, Ordering::AcqRel);
        self.moved_on();
    }

    fn moved_on(&self) {
        if self.debt.terminal.load(Ordering::Acquire) {
            self.announce();
        }
    }

    fn announce(&self) {
        self.progress.send_modify(|version| *version += 1);
    }

    pub(crate) async fn finished(&self) {
        let mut progress = self.progress.subscribe();
        let _ = progress.wait_for(|_| self.debt.quiet()).await;
    }
}

// Written into the boxes the host is given a pointer to, and checked before either is read back:
// what comes back over the ABI is whatever the host passed, and the tag is all that tells a buffer
// from a payload, or either from a pointer this library never handed out.
const LENT_TAG: u64 = 0x414b_5f4c_454e_5400;
const PAYLOAD_TAG: u64 = 0x414b_5f50_4159_4c00;

/// A buffer the host is filling.
///
/// It owns its call, as `Payload` does. The bytes are charged to the runtime's ledger until the
/// buffer comes back, and only the call can take that charge off; a weak reference that failed to
/// upgrade would be a charge no one is left to release.
#[repr(C)]
pub(crate) struct Lent {
    tag: u64,
    call: Arc<CallState>,
    data: Vec<u8>,
}

impl Lent {
    pub(crate) fn call(&self) -> &Arc<CallState> {
        &self.call
    }
}

pub(crate) fn keep(lent: Box<Lent>, status: ak_status) -> ak_status {
    let _ = Box::into_raw(lent);
    status
}

/// Takes back a box this library lent, named by the pointer it handed the host.
///
/// The tag tells a buffer from a payload, and either from a pointer that is neither. What it
/// cannot tell is a box already taken back: the read below happens before the check, so a second
/// return reads eight bytes out of a freed allocation, and whether the tag survived there is the
/// allocator's business. The header says a second return is undefined behaviour rather than a
/// no-op, because that is what it is - a token would be needed to make it reportable, and the
/// event path is where that token would be looked up.
///
/// Unaligned, because nothing promises the host's pointer is aligned for a `u64` - it is aligned
/// for whatever the host thinks `owner` points at, which is `void`.
///
/// # Safety
///
/// `owner` must be null, or a pointer this library handed out and the host has not given back.
unsafe fn take_tagged<T>(owner: *mut c_void, tag: u64) -> Option<Box<T>> {
    if owner.is_null() {
        return None;
    }
    if unsafe { owner.cast::<u64>().read_unaligned() } != tag {
        return None;
    }
    Some(unsafe { Box::from_raw(owner as *mut T) })
}

pub(crate) unsafe fn take_lent(owner: *mut c_void) -> Option<Box<Lent>> {
    unsafe { take_tagged(owner, LENT_TAG) }
}

#[repr(C)]
pub(crate) struct Payload {
    tag: u64,
    data: Bytes,
    call: Arc<CallState>,
    returns_credit: bool,
}

impl Drop for Payload {
    fn drop(&mut self) {
        self.call.payload_returned(self.returns_credit);
    }
}

pub(crate) unsafe fn take_payload(owner: *mut c_void) -> Option<Box<Payload>> {
    unsafe { take_tagged(owner, PAYLOAD_TAG) }
}

fn arena(len: usize) -> Result<Vec<u8>, ak_status> {
    let mut data = Vec::new();
    data.try_reserve_exact(len)
        .map_err(|_| ak_status::AK_STATUS_INTERNAL)?;
    data.resize(len, 0);
    Ok(data)
}

fn lend_payload(call: &Arc<CallState>, data: Bytes, returns_credit: bool) -> ak_bytes {
    call.debt.payloads.fetch_add(1, Ordering::AcqRel);
    call.ledger.hold();

    let payload = Box::new(Payload {
        tag: PAYLOAD_TAG,
        data,
        call: Arc::clone(call),
        returns_credit,
    });
    let ptr = payload.data.as_ptr();
    let len = payload.data.len();
    ak_bytes {
        ptr,
        len,
        owner: Box::into_raw(payload) as *mut c_void,
    }
}

fn create(
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
        ended_sending: AtomicBool::new(false),
        queueing: Mutex::new(()),
        channel,
        handle,
    });
    (state, rx)
}

fn start(
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

/// The channel's count, given back unless the call that took it is started.
///
/// Every exit from `start_on` between the join and the call being live has to give it back, an
/// unwind included: the entry point above turns a panic into AK_STATUS_INTERNAL, and a count left
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
        let (state, commands) = create(
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
    start(&state, send, recv, commands, services.spawner);
    Ok(handle)
}
