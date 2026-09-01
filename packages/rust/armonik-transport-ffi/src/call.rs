//! One call, as an actor: the task that delivers its events and the task that acquits its sends.
//!
//! The two are deliberately separate. A data callback and a WRITE_DONE for the same call may run
//! at once, so acquitting a send is never held behind a slow message handler; what the ABI
//! serializes is the data callbacks among themselves.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock, Weak};

use armonik_transport::grpc::{CallControl, GrpcStatusCode, RecvHalf, RecvResult, SendHalf};
use bytes::Bytes;
use tokio::sync::{mpsc, oneshot, watch, Semaphore};

use crate::abi::{ak_buffer, ak_bytes, ak_call_debt, ak_event_kind, ak_handle, ak_status};
use crate::blob;
use crate::host::{Host, HostPtr};
use crate::runtime::{AkRuntime, Ledger};

/// How many delivered payloads one call may have unconsumed at once.
pub(crate) const DELIVERY_CREDITS: usize = 1;

/// What the writing side hands the actor.
pub(crate) enum Command {
    Send(Bytes),
    EndSend,
}

/// The buffers a call may have out at once.
///
/// A slot is charged when a buffer is lent rather than when its message is committed, because the
/// allocation is what costs memory; committing moves a buffer from one side of the count to the
/// other without changing it.
struct SendWindow {
    occupied: AtomicU32,
    max: u32,
}

impl SendWindow {
    /// Takes a slot, or reports the window full. This bounded compare-and-swap is the model's
    /// `HasFreeSendSlot` guard, and a lend linearizes at its success.
    fn take(&self) -> bool {
        let mut seen = self.occupied.load(Ordering::Acquire);
        loop {
            if seen >= self.max {
                return false;
            }
            match self.occupied.compare_exchange_weak(
                seen,
                seen + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(current) => seen = current,
            }
        }
    }

    fn release(&self) {
        self.occupied.fetch_sub(1, Ordering::AcqRel);
    }
}

/// What is outstanding on a call, which is what decides when it can be reclaimed.
#[derive(Default)]
struct Debt {
    payloads: AtomicU32,
    buffers: AtomicU32,
    callbacks: AtomicU32,
    terminal: AtomicBool,
}

/// Everything a call's two tasks and the downcalls on it share.
pub(crate) struct CallState {
    ctx: HostPtr,
    host: Arc<Host>,
    ledger: Arc<Ledger>,
    control: CallControl,
    commands: mpsc::Sender<Command>,
    window: SendWindow,
    credits: Arc<Semaphore>,
    debt: Debt,
    /// Set by `ak_call_cancel`, observed by the reader: a call waiting on a delivery credit has
    /// to be able to give up waiting and go to its terminal.
    cancelled: AtomicBool,
    /// Bumped whenever the debt or the cancellation changes. A version and not a `Notify`: a
    /// waiter that checks its condition before creating the future misses a `notify_waiters`
    /// landing in between, and here that costs the call its terminal or its reclamation forever.
    progress: watch::Sender<u64>,
    /// Raised once the reader holds the terminal, which is what tells the writer to drain.
    over: watch::Sender<bool>,
    /// Raised by the half-close, so a send after it is refused rather than silently dropped and
    /// then acquitted as if it had gone out.
    ended_sending: AtomicBool,
    runtime: Weak<AkRuntime>,
    handle: OnceLock<ak_handle>,
    /// This state, for the buffers it lends: a buffer determines its call.
    me: Weak<CallState>,
}

impl CallState {
    pub(crate) fn debt(&self) -> ak_call_debt {
        ak_call_debt {
            payloads_owed: self.debt.payloads.load(Ordering::Acquire),
            buffers_lent: self.debt.buffers.load(Ordering::Acquire),
            callbacks_in_flight: self.debt.callbacks.load(Ordering::Acquire),
            terminal_delivered: self.debt.terminal.load(Ordering::Acquire) as i32,
        }
    }

    pub(crate) fn belongs_to(&self, runtime: &Weak<AkRuntime>) -> bool {
        Weak::ptr_eq(&self.runtime, runtime)
    }

    pub(crate) fn handle(&self) -> ak_handle {
        self.handle.get().copied().unwrap_or_default()
    }

    /// Asks the call to stop. The request takes effect when the actor observes it, which is why
    /// callbacks already committed may still arrive after this returns.
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.moved_on();
        self.control.cancel();
    }

    fn ended_sending(&self) -> bool {
        self.ended_sending.load(Ordering::Acquire)
    }

    /// Whether the call still takes commands from the host.
    fn live(&self) -> bool {
        !*self.over.borrow() && !self.debt.terminal.load(Ordering::Acquire)
    }

    /// Lends a buffer out of the call's arena, or says why it cannot.
    pub(crate) fn lend(&self, len: usize) -> Result<ak_buffer, ak_status> {
        if !self.live() || self.cancelled.load(Ordering::Acquire) {
            return Err(ak_status::AK_STATUS_INVALID_STATE);
        }
        // Permanent before transient: a request past the ceiling itself will never fit, and
        // saying so before the window is what stops a host retrying forever.
        self.ledger.could_ever_fit(len)?;
        if !self.window.take() {
            return Err(ak_status::AK_STATUS_SLOT_BUSY);
        }
        if let Err(status) = self.ledger.reserve(len) {
            self.window.release();
            return Err(status);
        }

        self.debt.buffers.fetch_add(1, Ordering::AcqRel);

        let mut lent = Box::new(Lent {
            tag: LENT_TAG,
            call: Weak::clone(&self.me),
            data: vec![0u8; len],
        });
        let ptr = lent.data.as_mut_ptr();
        Ok(ak_buffer {
            ptr,
            len,
            owner: Box::into_raw(lent) as *mut c_void,
        })
    }

    /// Accounts for a buffer coming back, whichever way it comes.
    fn took_back(&self, len: usize) {
        self.debt.buffers.fetch_sub(1, Ordering::AcqRel);
        self.ledger.release_bytes(len);
        self.moved_on();
    }

    /// Commits a lent buffer as the next message. Its slot stays charged until its WRITE_DONE.
    ///
    /// A refusal takes nothing back: the header says the buffer must then go out through
    /// `ak_return_call_buffer`, so freeing it here would leave the host holding a pointer to
    /// memory this side had already released.
    pub(crate) fn commit(&self, lent: Box<Lent>) -> ak_status {
        if !self.live() || self.cancelled.load(Ordering::Acquire) || self.ended_sending() {
            let _ = Box::into_raw(lent);
            return ak_status::AK_STATUS_INVALID_STATE;
        }

        let len = lent.data.len();
        match self
            .commands
            .try_send(Command::Send(Bytes::from(lent.data)))
        {
            Ok(()) => {
                self.took_back(len);
                ak_status::AK_STATUS_OK
            }
            Err(command) => {
                // The actor is gone under this downcall. The buffer goes back to the host, which
                // still has the one exit the header names.
                let Command::Send(bytes) = command.into_inner() else {
                    unreachable!("a send command carries its bytes")
                };
                let _ = Box::into_raw(Box::new(Lent {
                    tag: LENT_TAG,
                    call: Weak::clone(&self.me),
                    data: bytes.to_vec(),
                }));
                ak_status::AK_STATUS_INVALID_STATE
            }
        }
    }

    #[allow(clippy::boxed_local)]
    pub(crate) fn give_back(&self, lent: Box<Lent>) {
        // The box is the allocation coming back from the host; dropping it here is the point.
        self.took_back(lent.data.len());
        self.window.release();
    }

    pub(crate) fn end_send(&self) -> ak_status {
        if !self.live() || self.ended_sending.swap(true, Ordering::AcqRel) {
            return ak_status::AK_STATUS_INVALID_STATE;
        }
        match self.commands.try_send(Command::EndSend) {
            Ok(()) => ak_status::AK_STATUS_OK,
            Err(_) => ak_status::AK_STATUS_INVALID_STATE,
        }
    }

    fn moved_on(&self) {
        self.progress.send_modify(|version| *version += 1);
    }

    /// Resolves once the call has delivered its terminal and none of its callbacks is running.
    /// What the host still holds is a separate question, which reclamation asks.
    pub(crate) async fn finished(&self) {
        let mut progress = self.progress.subscribe();
        while !(self.debt.terminal.load(Ordering::Acquire)
            && self.debt.callbacks.load(Ordering::Acquire) == 0)
        {
            if progress.changed().await.is_err() {
                return;
            }
        }
    }

    fn settled(&self) -> bool {
        let debt = self.debt();
        debt.terminal_delivered == 1
            && debt.payloads_owed == 0
            && debt.buffers_lent == 0
            && debt.callbacks_in_flight == 0
    }
}

/// What the head of every allocation this ABI hands out says it is.
///
/// `ak_bytes` and `ak_buffer` have the same layout, so a host that gives one back through the
/// other's entry point would otherwise be a silent type confusion. Reading the tag turns the
/// realistic mistake - both are allocations of ours - into a refusal.
const LENT_TAG: u64 = 0x414b_5f4c_454e_5400;
const PAYLOAD_TAG: u64 = 0x414b_5f50_4159_4c00;

/// A buffer the host is filling.
#[repr(C)]
pub(crate) struct Lent {
    tag: u64,
    /// Which call lent it. The buffer determines its call, so nothing has to be told twice and a
    /// mismatched pair cannot move the wrong call's counters.
    call: Weak<CallState>,
    data: Vec<u8>,
}

impl Lent {
    /// The call that lent it, if that call is still there.
    pub(crate) fn call(&self) -> Option<Arc<CallState>> {
        self.call.upgrade()
    }
}

/// Takes a lent buffer back from the host, if that is what the pointer names.
///
/// # Safety
///
/// `owner` must be an allocation this ABI handed out, or null.
pub(crate) unsafe fn take_lent(owner: *mut c_void) -> Option<Box<Lent>> {
    if owner.is_null() {
        return None;
    }
    // SAFETY: every allocation this ABI hands out starts with its tag, so this read is in bounds
    // for either of them; a pointer that is neither is a host bug this cannot catch.
    if unsafe { *(owner as *const u64) } != LENT_TAG {
        return None;
    }
    // SAFETY: the tag says this is a `Lent` and the ABI says it has not been given back.
    Some(unsafe { Box::from_raw(owner as *mut Lent) })
}

/// A payload the host holds until it consumes it.
///
/// Everything a consumption has to undo lives here and is undone by dropping, so no path through
/// `ak_event_consumed` can do half of it.
#[repr(C)]
pub(crate) struct Payload {
    tag: u64,
    data: Vec<u8>,
    call: Arc<CallState>,
    /// The terminal takes no delivery credit, so it carries none back.
    returns_credit: bool,
}

impl Drop for Payload {
    fn drop(&mut self) {
        if self.returns_credit {
            self.call.credits.add_permits(1);
        }
        self.call.debt.payloads.fetch_sub(1, Ordering::AcqRel);
        self.call.ledger.release();
        self.call.moved_on();
    }
}

/// Takes a delivered payload back from the host, if that is what the pointer names.
///
/// # Safety
///
/// `owner` must be an allocation this ABI handed out, or null.
pub(crate) unsafe fn take_payload(owner: *mut c_void) -> Option<Box<Payload>> {
    if owner.is_null() {
        return None;
    }
    // SAFETY: every allocation this ABI hands out starts with its tag.
    if unsafe { *(owner as *const u64) } != PAYLOAD_TAG {
        return None;
    }
    // SAFETY: the tag says this is a `Payload` and the ABI says it has not been consumed.
    Some(unsafe { Box::from_raw(owner as *mut Payload) })
}

/// Hands `data` to the host as an owned payload, and records the debt that creates.
fn lend_payload(call: &Arc<CallState>, data: Vec<u8>, returns_credit: bool) -> ak_bytes {
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

/// Builds the shared state of a call. The tasks start once the handle names it.
pub(crate) fn create(
    ctx: HostPtr,
    runtime: &Arc<AkRuntime>,
    control: CallControl,
    max_sends_in_flight: u32,
    delivery_credits: usize,
) -> (Arc<CallState>, mpsc::Receiver<Command>) {
    // One slot per buffer the window allows: a command is only ever queued for a buffer that
    // took a slot, so the queue cannot be what refuses a commit.
    let (tx, rx) = mpsc::channel(max_sends_in_flight as usize + 1);

    let state = Arc::new_cyclic(|me| CallState {
        ctx,
        host: Arc::clone(&runtime.host),
        ledger: Arc::clone(&runtime.ledger),
        control,
        commands: tx,
        window: SendWindow {
            occupied: AtomicU32::new(0),
            max: max_sends_in_flight,
        },
        credits: Arc::new(Semaphore::new(delivery_credits)),
        debt: Debt::default(),
        cancelled: AtomicBool::new(false),
        progress: watch::channel(0).0,
        over: watch::channel(false).0,
        ended_sending: AtomicBool::new(false),
        runtime: Arc::downgrade(runtime),
        handle: OnceLock::new(),
        me: Weak::clone(me),
    });
    (state, rx)
}

/// Starts the call's tasks, now that `handle` names it.
pub(crate) fn start(
    state: &Arc<CallState>,
    handle: ak_handle,
    send: SendHalf,
    recv: RecvHalf,
    commands: mpsc::Receiver<Command>,
    spawner: &tokio::runtime::Handle,
) {
    let _ = state.handle.set(handle);
    let (writer_done, writer_is_done) = oneshot::channel();

    spawner.spawn(writer(Arc::clone(state), send, commands, writer_done));
    spawner.spawn(reader(Arc::clone(state), recv, writer_is_done));
    spawner.spawn(reclaim(Arc::clone(state)));
}

/// Acquits each accepted send, in send order, whatever became of it on the wire.
async fn writer(
    state: Arc<CallState>,
    send: SendHalf,
    mut commands: mpsc::Receiver<Command>,
    done: oneshot::Sender<()>,
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
                    // The reader has the terminal. What is already queued still owes its
                    // acquittal, so the queue is closed and then drained rather than dropped.
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
                if let Some(half) = send.as_mut() {
                    let _ = half.send_message(bytes).await;
                }
                // The slot goes back on emission and not on the callback's return, so a host
                // woken by WRITE_DONE may ask for a buffer from inside the callback.
                state.window.release();
                state.debt.callbacks.fetch_add(1, Ordering::AcqRel);
                state
                    .host
                    .signal(state.ctx, ak_event_kind::AK_EVENT_WRITE_DONE);
                state.debt.callbacks.fetch_sub(1, Ordering::AcqRel);
                state.moved_on();
            }
            Command::EndSend => {
                if let Some(half) = send.take() {
                    let _ = half.end_send().await;
                }
            }
        }
    }

    let _ = done.send(());
}

/// Delivers the call's events, in order, one credit at a time.
async fn reader(state: Arc<CallState>, mut recv: RecvHalf, writer_is_done: oneshot::Receiver<()>) {
    // Exactly one INITIAL_METADATA per call, first, synthesized empty when the wire carried none.
    // The synthesized one is not free: it takes a credit like any other.
    let head = recv.recv_initial_metadata().await.unwrap_or_default();
    let delivered_head = deliver(
        &state,
        ak_event_kind::AK_EVENT_INITIAL_METADATA,
        blob::encode_metadata(&head),
    )
    .await;

    // Cancelled while waiting for a credit: what is not delivered is dropped, and the terminal is
    // what the host gets instead. Never a bare return - the writer waits on a flag only the
    // terminal path raises, and the call is reclaimed only once the terminal has been delivered.
    let status = if !delivered_head {
        recv.next_message().await.ok().and_then(terminal_of)
    } else {
        loop {
            match recv.next_message().await {
                Ok(RecvResult::Message(message)) => {
                    if !deliver(
                        &state,
                        ak_event_kind::AK_EVENT_MESSAGE,
                        message.data.to_vec(),
                    )
                    .await
                    {
                        break recv.next_message().await.ok().and_then(terminal_of);
                    }
                }
                Ok(RecvResult::End(status)) => break Some(status),
                // The engine ends every call it started with a status; this is the call being
                // abandoned under the reader, which is what CANCELLED means here.
                Err(_) => break None,
            }
        }
    };

    // The terminal waits for every WRITE_DONE of the call to have returned.
    state.over.send_replace(true);
    let _ = writer_is_done.await;

    let (code, message, trailers) = match status {
        Some(status) => (
            status.code as i32,
            status.message,
            blob::encode_metadata(&status.trailing_metadata),
        ),
        None => (
            GrpcStatusCode::Cancelled as i32,
            String::from("the call was cancelled"),
            blob::encode_metadata(&Default::default()),
        ),
    };

    let payload = lend_payload(&state, status_payload(&message, &trailers), false);
    state.debt.callbacks.fetch_add(1, Ordering::AcqRel);
    state
        .host
        .deliver(state.ctx, ak_event_kind::AK_EVENT_STATUS, payload, code);
    state.debt.terminal.store(true, Ordering::Release);
    state.debt.callbacks.fetch_sub(1, Ordering::AcqRel);
    state.moved_on();
}

fn terminal_of(result: RecvResult) -> Option<armonik_transport::grpc::GrpcStatus> {
    match result {
        RecvResult::End(status) => Some(status),
        RecvResult::Message(_) => None,
    }
}

/// The bytes of a terminal payload: the reason, then the trailing metadata.
fn status_payload(message: &str, trailers: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + message.len() + trailers.len());
    out.extend_from_slice(&(message.len() as u32).to_ne_bytes());
    out.extend_from_slice(message.as_bytes());
    out.extend_from_slice(trailers);
    out
}

/// Takes a delivery credit and hands one event over. `false` when the call was cancelled while
/// waiting, in which case nothing was delivered.
///
/// Never the terminal: that one goes out with the credits spent, which is what lets a call end
/// while the host still owes what it has been given.
async fn deliver(state: &Arc<CallState>, kind: ak_event_kind, data: Vec<u8>) -> bool {
    let credits = Arc::clone(&state.credits);
    let permit = tokio::select! {
        biased;
        permit = credits.acquire() => permit,
        () = wait_for_cancel(state) => return false,
    };
    match permit {
        // The credit comes back with the acquittal rather than with this permit.
        Ok(permit) => permit.forget(),
        Err(_) => return false,
    }

    let payload = lend_payload(state, data, true);
    state.debt.callbacks.fetch_add(1, Ordering::AcqRel);
    state.host.deliver(state.ctx, kind, payload, 0);
    state.debt.callbacks.fetch_sub(1, Ordering::AcqRel);
    state.moved_on();
    true
}

async fn wait_for_cancel(state: &Arc<CallState>) {
    let mut progress = state.progress.subscribe();
    loop {
        if state.cancelled.load(Ordering::Acquire) {
            return;
        }
        if progress.changed().await.is_err() {
            return;
        }
    }
}

/// Reclaims the call once nothing of it is outstanding.
///
/// Not a downcall: every resource a call lends out is given back through something the runtime
/// already observes, so it knows when a terminal call owes nothing and takes the handle back
/// itself.
async fn reclaim(state: Arc<CallState>) {
    let mut progress = state.progress.subscribe();
    while !state.settled() {
        if progress.changed().await.is_err() {
            return;
        }
    }
    if let (Some(runtime), Some(handle)) = (state.runtime.upgrade(), state.handle.get()) {
        runtime.reclaim_call(*handle);
    }
}
