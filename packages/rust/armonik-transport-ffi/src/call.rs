//! One call, as an actor: the task that delivers its events and the task that acquits its sends.
//!
//! The two are deliberately separate. A data callback and a WRITE_DONE for the same call may run
//! at once, so acquitting a send is never held behind a slow message handler; what the ABI
//! serializes is the data callbacks among themselves.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Weak};

use armonik_transport::grpc::{
    CallControl, CallStartOptions, GrpcStatus, Metadata, RecvHalf, RecvResult, SendHalf,
};
use bytes::Bytes;
use tokio::sync::{mpsc, oneshot, watch, Semaphore};

use crate::abi::{ak_buffer, ak_bytes, ak_call_debt, ak_event_kind, ak_handle, ak_status};
use crate::blob;
use crate::channel::AkChannel;
use crate::host::{Host, HostPtr};
use crate::ledger::Ledger;
use crate::tables;

/// What a call needs from whoever runs it.
///
/// Declared here, by the consumer, and supplied by the runtime: a call needs somewhere to deliver
/// events, something to charge its bytes against, and threads to run on. It does not need the
/// object that happens to own those three, and naming one would put this module and the
/// runtime's in a cycle.
pub(crate) struct CallServices<'a> {
    pub(crate) host: &'a Arc<Host>,
    pub(crate) ledger: &'a Arc<Ledger>,
    pub(crate) spawner: &'a tokio::runtime::Handle,
}

/// What the writing side hands the actor.
pub(crate) enum Command {
    Send(Bytes),
    EndSend,
}

#[derive(Default)]
/// What is outstanding on a call, which is what decides when it can be reclaimed.
struct Debt {
    payloads: AtomicU32,
    buffers: AtomicU32,
    callbacks: AtomicU32,
    terminal: AtomicBool,
}

impl Debt {
    /// The terminal has gone out and nothing of the call is on the host's stack. What the host
    /// still holds is a separate question, which [`Debt::settled`] asks.
    fn quiet(&self) -> bool {
        self.terminal.load(Ordering::Acquire) && self.callbacks.load(Ordering::Acquire) == 0
    }

    /// Nothing of the call is outstanding at all: past its terminal, with every payload consumed
    /// and every buffer given back.
    fn settled(&self) -> bool {
        self.quiet()
            && self.payloads.load(Ordering::Acquire) == 0
            && self.buffers.load(Ordering::Acquire) == 0
    }

    /// The same four counters as the host reads them.
    fn as_abi(&self) -> ak_call_debt {
        ak_call_debt {
            payloads_owed: self.payloads.load(Ordering::Acquire),
            buffers_lent: self.buffers.load(Ordering::Acquire),
            callbacks_in_flight: self.callbacks.load(Ordering::Acquire),
            terminal_delivered: self.terminal.load(Ordering::Acquire) as i32,
        }
    }
}

/// Everything a call's two tasks and the downcalls on it share.
pub(crate) struct CallState {
    ctx: HostPtr,
    host: Arc<Host>,
    ledger: Arc<Ledger>,
    control: CallControl,
    commands: mpsc::Sender<Command>,
    /// The buffers the call may have out at once. A permit is taken when a buffer is lent rather
    /// than when its message is committed, because the allocation is what costs memory;
    /// committing moves a buffer from one side of the count to the other without changing it.
    /// Taking one is the model's `HasFreeSendSlot` guard, and a lend linearizes at its success.
    window: Semaphore,
    /// The payloads the call may have out at once: the delivery window, and the send window's
    /// mirror. A credit is spent by the deliverer and given back by the payload's destructor.
    credits: Semaphore,
    debt: Debt,
    /// Set by `ak_call_cancel`, observed by the reader: a call waiting on a delivery credit
    /// has to be able to give up waiting and go to its terminal.
    cancelled: AtomicBool,
    /// Bumped whenever the debt or the cancellation changes. A version and not a `Notify`:
    /// a waiter that checks its condition before creating the future misses one landing in
    /// between, and here that costs the call its terminal or its reclamation for good.
    progress: watch::Sender<u64>,
    /// Raised once the reader holds the terminal, which is what tells the writer to drain.
    over: watch::Sender<bool>,
    /// Raised by the half-close, so a send after it is refused rather than dropped and then
    /// acquitted as if it had gone out.
    ended_sending: AtomicBool,
    handle: ak_handle,
    /// Which channel started it. A channel closing cancels its own calls and no others.
    channel: ak_handle,
}

impl CallState {
    pub(crate) fn debt(&self) -> ak_call_debt {
        self.debt.as_abi()
    }

    pub(crate) fn belongs_to_channel(&self, channel: ak_handle) -> bool {
        self.channel == channel
    }

    /// Asks the call to stop. The request takes effect when the actor observes it, which is why
    /// callbacks already committed may still arrive after this returns.
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        // Announced whatever the call's state: `wait_for_cancel` is the one waiter the terminal
        // does not gate, and a reader parked on a delivery credit is reachable only this way.
        self.announce();
        self.control.cancel();
    }

    fn ended_sending(&self) -> bool {
        self.ended_sending.load(Ordering::Acquire)
    }

    /// Whether this call still takes work from the host.
    ///
    /// The shared half of what `lend` and `commit` refuse on; each adds what is its own -
    /// `commit` the end of sending, which a lend does not care about.
    fn accepts_work(&self) -> bool {
        self.live() && !self.cancelled.load(Ordering::Acquire)
    }

    /// Whether the call still takes commands from the host.
    fn live(&self) -> bool {
        !*self.over.borrow()
    }

    /// Lends a buffer out of the call's arena, or says why it cannot.
    pub(crate) fn lend(self: &Arc<Self>, len: usize) -> Result<ak_buffer, ak_status> {
        if !self.accepts_work() {
            return Err(ak_status::AK_STATUS_INVALID_STATE);
        }
        // One unfilled buffer at a time, whatever the window's depth: a host eligible to ask
        // holds nothing, which is what makes the window in-flight sends only and SLOT_BUSY's
        // wake-up its next WRITE_DONE. The depth buys pipelining of committed sends, not of
        // buffers being filled.
        if self.debt.buffers.load(Ordering::Acquire) > 0 {
            return Err(ak_status::AK_STATUS_INVALID_STATE);
        }
        // Permanent before transient: a request past the ceiling itself will never fit, and
        // saying so before the window is what stops a host retrying forever.
        self.ledger.could_ever_fit(len)?;
        let Ok(slot) = self.window.try_acquire() else {
            return Err(ak_status::AK_STATUS_SLOT_BUSY);
        };
        // The slot is only kept once the bytes are there, so a refused reservation leaves the
        // window as it found it rather than as something to put back.
        self.ledger.hold_bytes(len)?;
        slot.forget();

        self.debt.buffers.fetch_add(1, Ordering::AcqRel);

        let mut lent = Box::new(Lent {
            tag: LENT_TAG,
            call: Arc::downgrade(self),
            data: arena(len)?,
        });
        let ptr = lent.data.as_mut_ptr();
        Ok(ak_buffer {
            ptr,
            len,
            owner: Box::into_raw(lent) as *mut c_void,
        })
    }

    /// Accounts for a buffer coming back unused, bytes and all.
    fn took_back(&self, len: usize) {
        self.debt.buffers.fetch_sub(1, Ordering::AcqRel);
        self.ledger.release_bytes(len);
        self.moved_on();
    }

    /// Accounts for a buffer the host committed: it holds it no longer, but the bytes are still
    /// occupied - they moved from the host to this library, and only the send's acquittal ends
    /// them. A ceiling that fell here would be bounding what the host holds rather than what is
    /// outstanding.
    fn handed_over(&self) {
        self.debt.buffers.fetch_sub(1, Ordering::AcqRel);
        self.moved_on();
    }

    /// Accounts for a payload the host consumed: the credit it took, the debt it was, and its
    /// place in the count. The bytes weigh nothing against the ceiling, so none come back.
    fn payload_returned(&self, returns_credit: bool) {
        if returns_credit {
            self.credits.add_permits(1);
        }
        self.debt.payloads.fetch_sub(1, Ordering::AcqRel);
        self.ledger.release();
        self.moved_on();
    }

    /// Commits a lent buffer as the next message. Its slot stays charged until its WRITE_DONE.
    pub(crate) fn commit(self: &Arc<Self>, lent: Box<Lent>) -> ak_status {
        if !self.accepts_work() || self.ended_sending() {
            return keep(lent, ak_status::AK_STATUS_INVALID_STATE);
        }

        // Reserved before the box is consumed, so a refusal leaves the host holding the very
        // allocation it was lent rather than a copy of its bytes.
        let Ok(slot) = self.commands.try_reserve() else {
            return keep(lent, ak_status::AK_STATUS_INVALID_STATE);
        };
        self.handed_over();
        slot.send(Command::Send(Bytes::from(lent.data)));
        ak_status::AK_STATUS_OK
    }

    #[allow(clippy::boxed_local)]
    /// Gives a lent buffer back unused. Legal on a cancelled or terminal call: it is the only
    /// exit for a buffer whose send is refused, and the call is not reclaimed until it happens.
    pub(crate) fn give_back(&self, lent: Box<Lent>) {
        self.took_back(lent.data.len());
        self.window.add_permits(1);
    }

    pub(crate) fn end_send(&self) -> ak_status {
        if !self.live() {
            return ak_status::AK_STATUS_INVALID_STATE;
        }
        // The room comes before the latch, as it does for a commit: the queue is sized so this
        // cannot be what refuses, and reserving first leaves no state observable as ended and
        // then not.
        let Ok(slot) = self.commands.try_reserve() else {
            return ak_status::AK_STATUS_INVALID_STATE;
        };
        if self.ended_sending.swap(true, Ordering::AcqRel) {
            return ak_status::AK_STATUS_INVALID_STATE;
        }
        slot.send(Command::EndSend);
        ak_status::AK_STATUS_OK
    }

    /// Runs `emit` with this call counted as having a callback on the stack, which is what the
    /// terminal and the shutdown both wait on.
    fn in_callback(&self, emit: impl FnOnce()) {
        self.debt.callbacks.fetch_add(1, Ordering::AcqRel);
        emit();
        self.debt.callbacks.fetch_sub(1, Ordering::AcqRel);
        self.moved_on();
    }

    /// Announces a change to whoever waits on this call, once there is anything to conclude.
    ///
    /// Silent before the terminal, deliberately: both remaining waiters ask `quiet` or `settled`,
    /// and each of those requires the terminal, which is stored before the callback count that
    /// completes them. So a bump for a message in flight could only wake a task to look and go
    /// back to sleep - and with a delivery window of one, the reader is parked for most of a
    /// call, so that was three wake-ups per message pair charged to the host's own threads.
    fn moved_on(&self) {
        if self.debt.terminal.load(Ordering::Acquire) {
            self.announce();
        }
    }

    fn announce(&self) {
        self.progress.send_modify(|version| *version += 1);
    }

    /// Resolves once the call has delivered its terminal and none of its callbacks is running.
    pub(crate) async fn finished(&self) {
        let mut progress = self.progress.subscribe();
        let _ = progress.wait_for(|_| self.debt.quiet()).await;
    }
}

/// What the head of every allocation this ABI hands out says it is.
///
/// `ak_bytes` and `ak_buffer` have the same layout, so a host that gives one back through the
/// other's entry point would otherwise be a silent type confusion.
const LENT_TAG: u64 = 0x414b_5f4c_454e_5400;
const PAYLOAD_TAG: u64 = 0x414b_5f50_4159_4c00;

#[repr(C)]
/// A buffer the host is filling.
pub(crate) struct Lent {
    tag: u64,
    /// Which call lent it. The buffer determines its call, so nothing has to be told twice
    /// and a mismatched pair cannot move the wrong call's counters.
    call: Weak<CallState>,
    data: Vec<u8>,
}

impl Lent {
    pub(crate) fn call(&self) -> Option<Arc<CallState>> {
        self.call.upgrade()
    }
}

/// Answers `status` and leaves the buffer with the host.
///
/// Every refusal on the send path goes through here. The header says a refused buffer must still
/// go out through `ak_return_call_buffer`, so freeing it would leave the host holding a pointer
/// into memory this side had already released.
pub(crate) fn keep(lent: Box<Lent>, status: ak_status) -> ak_status {
    let _ = Box::into_raw(lent);
    status
}

/// Takes an allocation back from the host, if the tag says it is the one expected.
///
/// # Safety
///
/// `owner` must be an allocation this ABI handed out, or null.
unsafe fn take_tagged<T>(owner: *mut c_void, tag: u64) -> Option<Box<T>> {
    if owner.is_null() {
        return None;
    }
    // SAFETY: every allocation this ABI hands out starts with its tag, so this read is in
    // bounds for either of them; a pointer that is neither is a host bug this cannot catch.
    if unsafe { *(owner as *const u64) } != tag {
        return None;
    }
    Some(unsafe { Box::from_raw(owner as *mut T) })
}

/// Takes a lent buffer back from the host, if that is what the pointer names.
///
/// # Safety
///
/// `owner` must be an allocation this ABI handed out, or null.
pub(crate) unsafe fn take_lent(owner: *mut c_void) -> Option<Box<Lent>> {
    unsafe { take_tagged(owner, LENT_TAG) }
}

#[repr(C)]
/// A payload the host holds until it consumes it.
///
/// Everything a consumption has to undo lives here and is undone by dropping, so no path through
/// `ak_event_consumed` can do half of it.
pub(crate) struct Payload {
    tag: u64,
    /// The message as it arrived, and not a copy of it: a received message is already a view on
    /// the buffer the session read into, and the host reads it without owning it.
    data: Bytes,
    call: Arc<CallState>,
    /// The terminal takes no delivery credit, so it carries none back.
    returns_credit: bool,
}

impl Drop for Payload {
    fn drop(&mut self) {
        self.call.payload_returned(self.returns_credit);
    }
}

/// Takes a delivered payload back from the host, if that is what the pointer names.
///
/// # Safety
///
/// `owner` must be an allocation this ABI handed out, or null.
pub(crate) unsafe fn take_payload(owner: *mut c_void) -> Option<Box<Payload>> {
    unsafe { take_tagged(owner, PAYLOAD_TAG) }
}

/// Hands `data` to the host as an owned payload, and records the debt that creates.
/// `len` writable bytes for the host, or why there are none.
///
/// Through the fallible API, because the specification promises `AK_STATUS_INTERNAL` for a
/// failure here: `vec![0u8; len]` runs the allocation-error hook and aborts, and a library
/// loaded into somebody else's process has no business ending it.
///
/// Zeroed, though the header promises nothing about the contents: handing over uninitialised
/// heap would give the host a window onto this process's own memory, and the pass costs what the
/// host is about to spend writing the bytes anyway.
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

/// Builds the shared state of a call. The tasks start once it is published.
fn create(
    ctx: HostPtr,
    handle: ak_handle,
    channel: ak_handle,
    services: &CallServices<'_>,
    control: CallControl,
    max_sends_in_flight: usize,
    delivery_credits: usize,
) -> (Arc<CallState>, mpsc::Receiver<Command>) {
    // One slot per buffer the window allows: a command is only ever queued for a buffer that
    // took a slot, so the queue cannot be what refuses a commit.
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
        channel,
        handle,
    });
    (state, rx)
}

/// Starts the call's tasks.
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
                let charged = bytes.len();
                if let Some(half) = send.as_mut() {
                    let _ = half.send_message(bytes).await;
                }
                state.in_callback(|| {
                    // Inside the callback and before the signal: a host woken by WRITE_DONE may
                    // ask for a buffer from within it and finds the room its acquittal just
                    // freed, which is what the header promises. Releasing before entering the
                    // callback instead left another thread able to see a free slot and budget
                    // room for a send whose acquittal had not gone out - one buffer over the
                    // two bounds the model states over the emission.
                    state.window.add_permits(1);
                    state.ledger.release_bytes(charged);
                    state
                        .host
                        .signal(state.ctx, ak_event_kind::AK_EVENT_WRITE_DONE)
                });
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
    // Exactly one INITIAL_METADATA per call, first, synthesized empty when the wire carried
    // none. The synthesized one is not free: it takes a credit like any other.
    let head = match recv.recv_initial_metadata().await {
        Ok(metadata) => blob::encode_metadata(metadata),
        Err(_) => blob::encode_metadata(&Metadata::new()),
    };
    let delivered_head = deliver(
        &state,
        ak_event_kind::AK_EVENT_INITIAL_METADATA,
        Bytes::from(head),
    )
    .await;

    // A delivery that answers false was cancelled while waiting for a credit, and what it was
    // carrying is dropped. The terminal is then CANCELLED and not whatever the peer had decided:
    // the model admits no other terminal once cancellation is latched, and forwarding the
    // engine's status would tell the host the call completed while an event of it was thrown
    // away. Never a bare return - the writer waits on a flag only the terminal path raises.
    let status = if !delivered_head {
        GrpcStatus::cancelled()
    } else {
        loop {
            // Asked before every event, and not only when a delivery credit is scarce. `deliver`
            // consults the latch on the arm that waits, so with a credit in hand a message
            // already queued would go out after the cancellation, and the peer's own status
            // behind it - which reads to the host as a call that completed while an event of it
            // was dropped. What decides the terminal is the latch; a free credit is not a
            // licence to ignore it.
            if state.cancelled.load(Ordering::Acquire) {
                break GrpcStatus::cancelled();
            }

            match recv.next_message().await {
                Ok(RecvResult::Message(message)) => {
                    if !deliver(&state, ak_event_kind::AK_EVENT_MESSAGE, message.data).await {
                        break GrpcStatus::cancelled();
                    }
                }
                Ok(RecvResult::End(status)) => break status,
                // The engine ends every call it started with a status; this is the call
                // being abandoned under the reader, which is what CANCELLED means here.
                Err(_) => break GrpcStatus::cancelled(),
            }
        }
    };

    // The terminal waits for every WRITE_DONE of the call to have returned.
    state.over.send_replace(true);
    let _ = writer_is_done.await;

    // Scoped, so nothing holding the host's raw pointers is alive across the wait below: an
    // `ak_bytes` is not `Send`, and this task is spawned.
    {
        let payload = lend_payload(
            &state,
            Bytes::from(blob::status_payload(
                &status.message,
                &status.trailing_metadata,
            )),
            false,
        );
        state.in_callback(|| {
            state.host.deliver(
                state.ctx,
                ak_event_kind::AK_EVENT_STATUS,
                payload,
                status.code.as_i32(),
            );
            // Inside the callback's count and after the delivery: what makes `finished` correct
            // is that the callback has not returned yet, and what makes the terminal exactly
            // once is that nothing but this line sets it.
            state.debt.terminal.store(true, Ordering::Release);

            // And inside it too, so the channel stops counting this call before the callback
            // count that resolves `finished` drops. The shutdown waits on `finished`; doing this
            // after the callback returned let a runtime announce it had stopped while a channel
            // of it was still CLOSING, which is not a state the model admits of a drained one.
            crate::lifecycle::call_reached_terminal(state.channel);
        });
    }

    // Given up here and not at the end of the task: the reclamation below waits on the host, and
    // the engine's reading half has nothing left to do once the terminal is out.
    drop(recv);

    reclaim(&state).await;
}

/// Waits until nothing of the call is outstanding, and takes its handle back.
///
/// Not a downcall, and not a task of its own: every resource a call lends out comes back through
/// something the runtime already observes, and by here the reader has nothing else to do - it is
/// the very thing that delivered the terminal this waits behind.
async fn reclaim(state: &Arc<CallState>) {
    let mut progress = state.progress.subscribe();
    let _ = progress.wait_for(|_| state.debt.settled()).await;
    crate::lifecycle::call_settled(state.handle);
}

/// Takes a delivery credit and hands one event over. `false` when the call was cancelled while
/// waiting, in which case nothing was delivered.
///
/// Never the terminal: that one goes out with the credits spent, which is what lets a call end
/// while the host still owes what it has been given.
async fn deliver(state: &Arc<CallState>, kind: ak_event_kind, data: Bytes) -> bool {
    let permit = tokio::select! {
        biased;
        permit = state.credits.acquire() => permit,
        () = wait_for_cancel(state) => return false,
    };
    match permit {
        // The credit comes back with the acquittal rather than with this permit.
        Ok(permit) => permit.forget(),
        Err(_) => return false,
    }

    let payload = lend_payload(state, data, true);
    state.in_callback(|| state.host.deliver(state.ctx, kind, payload, 0));
    true
}

async fn wait_for_cancel(state: &CallState) {
    let mut progress = state.progress.subscribe();
    let _ = progress
        .wait_for(|_| state.cancelled.load(Ordering::Acquire))
        .await;
}

/// Starts a call on `channel` and publishes it, answering the handle the host will name it by.
///
/// The ordering here is the point: the call is published before its tasks run, because one that
/// ends at once would otherwise reach its reclamation, find the slot empty, and then be published
/// into it as a dead entry.
pub(crate) fn start_on(
    channel: &Arc<AkChannel>,
    channel_handle: ak_handle,
    services: &CallServices<'_>,
    method: &str,
    metadata: Metadata,
    ctx: HostPtr,
) -> Result<ak_handle, ak_status> {
    // First, and it can refuse: a channel that is closing takes no new call. Taking the place
    // before anything is built is what makes the refusal free, and what makes the check
    // conclusive - the channel cannot close between here and the publish, because closing waits
    // on this place being given back.
    channel.join()?;

    let mut options = CallStartOptions::new(method);
    options.metadata = metadata;

    let grpc_call = match channel.grpc.start_call(options) {
        Ok(call) => call,
        Err(error) => {
            channel.leave();
            return Err(ak_status::from(error));
        }
    };

    let (send, recv, control) = grpc_call.split();
    let (handle, (state, commands)) = tables::calls().insert_with(|handle| {
        let (state, commands) = create(
            ctx,
            handle,
            channel_handle,
            services,
            control,
            channel.max_sends_in_flight,
            channel.delivery_credits,
        );
        (Arc::clone(&state), (state, commands))
    });

    start(&state, send, recv, commands, services.spawner);
    Ok(handle)
}
