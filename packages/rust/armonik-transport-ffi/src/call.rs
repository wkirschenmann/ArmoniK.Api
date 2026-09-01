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
use tokio::sync::{mpsc, oneshot, watch, Notify, Semaphore};

use crate::abi::{ak_buffer, ak_bytes, ak_call_debt, ak_event_kind, ak_handle, ak_status};
use crate::blob;
use crate::host::{Host, HostPtr};
use crate::runtime::{AkRuntime, Ledger};

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
    cancelled_or_settled: Notify,
    /// Raised once the reader holds the terminal, which is what tells the writer to drain.
    over: watch::Sender<bool>,
    runtime: Weak<AkRuntime>,
    handle: OnceLock<ak_handle>,
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
        self.cancelled_or_settled.notify_waiters();
        self.control.cancel();
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
        if !self.window.take() {
            return Err(ak_status::AK_STATUS_SLOT_BUSY);
        }

        self.debt.buffers.fetch_add(1, Ordering::AcqRel);
        self.ledger.hold();

        let mut lent = Box::new(Lent { data: vec![0u8; len] });
        let ptr = lent.data.as_mut_ptr();
        Ok(ak_buffer {
            ptr,
            len,
            owner: Box::into_raw(lent) as *mut c_void,
        })
    }

    /// Takes a lent buffer back, whichever way it is being given back.
    ///
    /// # Safety
    ///
    /// `owner` must name a buffer this call lent and has not taken back.
    unsafe fn reclaim(&self, owner: *mut c_void) -> Vec<u8> {
        // SAFETY: forwarded from this function's own contract.
        let lent = unsafe { Box::from_raw(owner as *mut Lent) };
        self.debt.buffers.fetch_sub(1, Ordering::AcqRel);
        self.ledger.release();
        self.settled_may_have_changed();
        lent.data
    }

    /// Commits a lent buffer as the next message. Its slot stays charged until its WRITE_DONE.
    ///
    /// # Safety
    ///
    /// `buffer` must be one this call lent and has not taken back.
    pub(crate) unsafe fn commit(&self, buffer: ak_buffer) -> ak_status {
        // SAFETY: forwarded from this function's own contract.
        let data = unsafe { self.reclaim(buffer.owner) };
        if !self.live() || self.cancelled.load(Ordering::Acquire) {
            self.window.release();
            return ak_status::AK_STATUS_INVALID_STATE;
        }

        match self.commands.try_send(Command::Send(Bytes::from(data))) {
            Ok(()) => ak_status::AK_STATUS_OK,
            Err(_) => {
                self.window.release();
                ak_status::AK_STATUS_INVALID_STATE
            }
        }
    }

    /// Gives a lent buffer back unused. Legal on a cancelled or terminal call: it is the only
    /// exit for a buffer whose send is refused, and the call is not reclaimed until it happens.
    ///
    /// # Safety
    ///
    /// `buffer` must be one this call lent and has not taken back.
    pub(crate) unsafe fn give_back(&self, buffer: ak_buffer) {
        // SAFETY: forwarded from this function's own contract.
        drop(unsafe { self.reclaim(buffer.owner) });
        self.window.release();
    }

    pub(crate) fn end_send(&self) -> ak_status {
        if !self.live() {
            return ak_status::AK_STATUS_INVALID_STATE;
        }
        match self.commands.try_send(Command::EndSend) {
            Ok(()) => ak_status::AK_STATUS_OK,
            Err(_) => ak_status::AK_STATUS_INVALID_STATE,
        }
    }

    fn settled_may_have_changed(&self) {
        self.cancelled_or_settled.notify_waiters();
    }

    /// Resolves once the call has delivered its terminal and none of its callbacks is running.
    /// What the host still holds is a separate question, which reclamation asks.
    pub(crate) async fn finished(&self) {
        while !(self.debt.terminal.load(Ordering::Acquire)
            && self.debt.callbacks.load(Ordering::Acquire) == 0)
        {
            self.cancelled_or_settled.notified().await;
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

/// A buffer the host is filling.
struct Lent {
    data: Vec<u8>,
}

/// A payload the host holds until it consumes it.
///
/// Everything a consumption has to undo lives here and is undone by dropping, so no path through
/// `ak_event_consumed` can do half of it.
pub(crate) struct Payload {
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
        self.call.settled_may_have_changed();
    }
}

/// Hands `data` to the host as an owned payload, and records the debt that creates.
fn lend_payload(call: &Arc<CallState>, data: Vec<u8>, returns_credit: bool) -> ak_bytes {
    call.debt.payloads.fetch_add(1, Ordering::AcqRel);
    call.ledger.hold();

    let payload = Box::new(Payload {
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

/// What a call needs to start, once its handle is known.
pub(crate) struct Halves {
    pub(crate) send: SendHalf,
    pub(crate) recv: RecvHalf,
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

    let state = Arc::new(CallState {
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
        cancelled_or_settled: Notify::new(),
        over: watch::channel(false).0,
        runtime: Arc::downgrade(runtime),
        handle: OnceLock::new(),
    });
    (state, rx)
}

/// Starts the call's tasks, now that `handle` names it.
pub(crate) fn start(
    state: &Arc<CallState>,
    handle: ak_handle,
    halves: Halves,
    commands: mpsc::Receiver<Command>,
    spawner: &tokio::runtime::Handle,
) {
    let _ = state.handle.set(handle);
    let (writer_done, writer_is_done) = oneshot::channel();

    spawner.spawn(writer(Arc::clone(state), halves.send, commands, writer_done));
    spawner.spawn(reader(Arc::clone(state), halves.recv, writer_is_done));
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
                state.host.signal(state.ctx, ak_event_kind::AK_EVENT_WRITE_DONE);
                state.debt.callbacks.fetch_sub(1, Ordering::AcqRel);
                state.settled_may_have_changed();
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
    if !deliver(
        &state,
        ak_event_kind::AK_EVENT_INITIAL_METADATA,
        blob::encode_metadata(&head),
        0,
    )
    .await
    {
        return;
    }

    let status = loop {
        match recv.next_message().await {
            Ok(RecvResult::Message(message)) => {
                if !deliver(
                    &state,
                    ak_event_kind::AK_EVENT_MESSAGE,
                    message.data.to_vec(),
                    0,
                )
                .await
                {
                    // Cancelled while waiting for a credit: what is not delivered is dropped, and
                    // the terminal is what the host gets instead.
                    break recv.next_message().await.ok().and_then(terminal_of);
                }
            }
            Ok(RecvResult::End(status)) => break Some(status),
            Err(_) => break None,
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
    state.host.deliver_status(state.ctx, code, payload);
    state.debt.terminal.store(true, Ordering::Release);
    state.debt.callbacks.fetch_sub(1, Ordering::AcqRel);
    state.settled_may_have_changed();
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
async fn deliver(
    state: &Arc<CallState>,
    kind: ak_event_kind,
    data: Vec<u8>,
    status_code: i32,
) -> bool {
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
    if kind == ak_event_kind::AK_EVENT_STATUS {
        state.host.deliver_status(state.ctx, status_code, payload);
    } else {
        state.host.deliver(state.ctx, kind, payload);
    }
    state.debt.callbacks.fetch_sub(1, Ordering::AcqRel);
    state.settled_may_have_changed();
    true
}

async fn wait_for_cancel(state: &Arc<CallState>) {
    loop {
        if state.cancelled.load(Ordering::Acquire) {
            return;
        }
        state.cancelled_or_settled.notified().await;
    }
}

/// Reclaims the call once nothing of it is outstanding.
///
/// Not a downcall: every resource a call lends out is given back through something the runtime
/// already observes, so it knows when a terminal call owes nothing and takes the handle back
/// itself.
async fn reclaim(state: Arc<CallState>) {
    while !state.settled() {
        state.cancelled_or_settled.notified().await;
    }
    if let (Some(runtime), Some(handle)) = (state.runtime.upgrade(), state.handle.get()) {
        runtime.reclaim_call(*handle);
    }
}
