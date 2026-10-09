use std::ffi::c_void;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use armonik_transport::grpc::{CallControl, CallDriver, FramedMessage, OneRequest, SendHalf};
use tokio::sync::{mpsc, watch, Semaphore};

use crate::abi::{ak_buffer, ak_call_debt, ak_handle, ak_status};
use crate::channel::AkChannel;
use crate::guard_void;
use crate::host::{Host, HostPtr};
use crate::ledger::{Ledger, Waiter};

mod actor;
mod lent;
mod start;
mod turn;

pub(crate) use lent::HEADROOM;
use lent::{arena, slack, LENT_TAG};
pub(crate) use lent::{take_lent, take_payload, Lent, Taken};
pub(crate) use start::{start_on, Shape};
use turn::ReadTurn;

pub(crate) struct CallServices<'a> {
    pub(crate) host: &'a Arc<Host>,
    pub(crate) ledger: &'a Arc<Ledger>,
}

/// Where a call's requests go: a stream's commands to its writer, or the one request of a call that
/// sends one.
pub(crate) enum Requests {
    Stream(mpsc::Sender<Command>),
    One(OneRequest),
}

/// What a call's one task is made of, until it is spawned.
pub(crate) struct CallTask {
    driver: CallDriver,
    /// A stream's writer: the transport's send half and the commands it takes. None on a call that
    /// sends one request, which has no writer.
    writing: Option<(SendHalf, mpsc::Receiver<Command>)>,
}

pub(crate) enum Command {
    /// A message, and the bytes its lend was charged, which it may not have filled.
    Send {
        message: FramedMessage,
        charged: usize,
    },
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
        // sees the terminal. The other reads here and in `settled` are sequentially consistent for
        // the wake-up `moved_on` states.
        self.terminal.load(Ordering::SeqCst) && self.callbacks.load(Ordering::SeqCst) == 0
    }

    fn settled(&self) -> bool {
        self.quiet()
            && self.payloads.load(Ordering::SeqCst) == 0
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
    requests: Requests,
    /// The call's task until it is spawned: at once for a stream; for a call that sends one request,
    /// by whichever comes first of its commit and what needs the task before it - a cancellation,
    /// and a lend refused for the budget, whose wake-up the task raises.
    task: Mutex<Option<CallTask>>,
    window: Semaphore,
    credits: Semaphore,
    debt: Debt,
    cancelled: AtomicBool,
    progress: watch::Sender<u64>,
    /// Set by the first reclaim. A call whose debt stays paid is announced again by each later
    /// `moved_on` - a lend refused on it, for one - and the table's lock is not taken again.
    reclaimed: AtomicBool,
    over: watch::Sender<bool>,
    /// Whether the sending has ended, in `SENDING_ENDED`, beside how many sends are being queued.
    ///
    /// The two writers are `ak_call_send_message` and `ak_call_end_send`, and the queue orders
    /// commands by when they are sent, not by when a slot was reserved. An end queued while a send
    /// was on its way to the queue would go in first: the writer takes the end, drops the send
    /// half, finds no half for the message, and emits its WRITE_DONE anyway - the host told a
    /// message left that never did. So a send counts itself in before it looks and out when its
    /// downcall returns, and the end takes the word only when it counts no send, waiting for one
    /// it finds: the WRITE_DONE of a send can reach the host before that send's downcall has
    /// returned, and a host ending from inside it is ending after the send, not beside it.
    sending: AtomicU32,
    /// The send this call has refused for room, and the wake-up a release owes it.
    waiter: Arc<Waiter>,
    turn: Arc<ReadTurn>,
    handle: ak_handle,
    // The channel itself, not its name: leaving it is not optional, and a name would make it
    // conditional on a lookup whose failure the reader has no answer for.
    channel: Arc<AkChannel>,
}

/// What a lend owes the call and the ledger, beside its claim of the call's one buffer.
#[derive(Clone, Copy)]
struct Owed {
    /// The bytes it charged and counted in the ledger, once it has.
    bytes: Option<usize>,
    /// The slot of the send window it spent.
    slot: bool,
}

impl Owed {
    /// A claim of the call's one buffer, and nothing else yet.
    fn claim() -> Self {
        Self {
            bytes: None,
            slot: false,
        }
    }

    /// A lend that has everything, charged `charged` bytes.
    fn lent(charged: usize) -> Self {
        Self {
            bytes: Some(charged),
            slot: true,
        }
    }
}

/// A lend that the host does not hold the buffer of yet, which pays what it owes when it is
/// dropped: a refusal drops it, and so does an unwind. `CallState::lend` forgets it once the
/// buffer is the host's.
struct Lending<'a> {
    call: &'a CallState,
    owed: Owed,
}

impl Drop for Lending<'_> {
    fn drop(&mut self) {
        self.call.repay(self.owed);
    }
}

/// A lend's record of its send as waiting for room, which ends the wait when it is dropped.
/// `CallState::charge_or_wait` forgets it when the record is to stand.
struct Waiting<'a> {
    call: &'a CallState,
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        guard_void(|| self.call.ledger.stop_waiting(&self.call.waiter));
    }
}

/// The bit of `CallState::sending` that says the sending has ended; the bits below count sends.
const SENDING_ENDED: u32 = 1 << 31;

/// A send counted in `CallState::sending` from its look at the word until its downcall returns.
struct Queueing<'a>(&'a AtomicU32);

impl<'a> Queueing<'a> {
    /// Counts a send in, unless the sending has ended.
    fn enter(sending: &'a AtomicU32) -> Option<Self> {
        let mut seen = sending.load(Ordering::Acquire);
        loop {
            if seen & SENDING_ENDED != 0 {
                return None;
            }
            match sending.compare_exchange_weak(seen, seen + 1, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return Some(Self(sending)),
                Err(now) => seen = now,
            }
        }
    }
}

impl Drop for Queueing<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl CallState {
    pub(crate) fn debt(&self) -> ak_call_debt {
        self.debt.as_abi()
    }

    #[cfg(feature = "test-hooks")]
    pub(crate) fn is_waiting_for_room(&self) -> bool {
        self.waiter.is_waiting()
    }

    pub(crate) fn cancel(self: &Arc<Self>) {
        self.cancelled.store(true, Ordering::Release);
        self.announce();
        self.control.cancel();
        // A call not yet spawned ends through its task, as any other.
        self.spawn_task();
    }

    /// Spawns the call's task, once: the first to ask takes it, under the lock that decides.
    fn spawn_task(self: &Arc<Self>) {
        let task = self
            .task
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(task) = task {
            actor::spawn(self, task);
        }
    }

    fn sends_one_request(&self) -> bool {
        matches!(self.requests, Requests::One(_))
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
    ///
    /// What the lend takes is owed back until the host has the buffer, and a refusal and a panic
    /// both pay it: the claim, the slot of the window and the bytes. A panic leaves a refused
    /// lend, which the entry point answers AK_STATUS_INTERNAL, as it does an allocator failure.
    pub(crate) fn lend(self: &Arc<Self>, len: usize) -> Result<ak_buffer, ak_status> {
        if self
            .debt
            .buffers
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(ak_status::AK_STATUS_INVALID_STATE);
        }
        let mut lending = Lending {
            call: self,
            owed: Owed::claim(),
        };
        at!(at_lend_step, LendStep::Claimed);

        // Claimed, then read, against a reader that publishes the terminal and then reads the
        // claim. Both in the sequentially consistent order, so the two cannot pass each other:
        // either the reader sees this claim and does not settle, or this sees the terminal.
        let lent = if self.debt.terminal.load(Ordering::SeqCst) {
            Err(ak_status::AK_STATUS_INVALID_STATE)
        } else {
            self.fill(len, &mut lending)
        };

        if lent.is_ok() {
            std::mem::forget(lending);
        }
        lent
    }

    /// What a buffer of `len` bytes asks of the call and of the ceiling's size, before any of its
    /// bytes: a lend and a resize alike.
    fn admits_buffer(&self, len: usize) -> Result<(), ak_status> {
        if !self.accepts_work() {
            return Err(ak_status::AK_STATUS_INVALID_STATE);
        }
        // Its one request committed, a call takes no other, and no WRITE_DONE will come for a
        // SLOT_BUSY to wait on.
        if self.sends_one_request() && self.sending.load(Ordering::Acquire) & SENDING_ENDED != 0 {
            return Err(ak_status::AK_STATUS_INVALID_STATE);
        }
        // The ceiling covers what the wire and the allocator can carry as well as what the host
        // budgeted: `Ledger` caps one by the other. Refused here rather than at the send, where
        // the engine's refusal is swallowed behind a WRITE_DONE and the host is told a message it
        // never sent has left.
        self.ledger.could_ever_fit(len)
    }

    fn fill(self: &Arc<Self>, len: usize, lending: &mut Lending) -> Result<ak_buffer, ak_status> {
        self.admits_buffer(len)?;
        at!(at_lend_step, LendStep::Admitted);
        // Spent at once and owed back by `lending`. Once the host has sent the message, its
        // WRITE_DONE gives the slot back.
        match self.window.try_acquire() {
            Ok(slot) => slot.forget(),
            Err(_) => return Err(ak_status::AK_STATUS_SLOT_BUSY),
        }
        lending.owed.slot = true;
        at!(at_lend_step, LendStep::Windowed);
        #[cfg(feature = "test-hooks")]
        crate::hooks::run_before_charge();
        self.charge_or_wait(len)?;
        // Served: reads are no longer held back for it. A wait that an earlier refusal of this
        // call left is ended here. Only a lend of this call records a wait, and one lend runs at
        // a time, so a wait this reads as absent is absent.
        lending.owed.bytes = Some(len);
        if self.waiter.is_waiting() {
            self.ledger.stop_waiting(&self.waiter);
        }
        at!(at_lend_step, LendStep::Charged);

        // Read again once counted: a shutdown that ran after the checks above found nothing
        // owed. The other half is `Debt::quiet` then `Ledger::empty`.
        if self.debt.terminal.load(Ordering::SeqCst) {
            return Err(ak_status::AK_STATUS_INVALID_STATE);
        }

        // The headroom is kept ahead of what the host writes, and the commit frames the message
        // in it.
        let mut data = arena(HEADROOM, len, Some(&self.channel.spares))?;
        at!(at_lend_step, LendStep::Allocated);
        // A lend is charged what backs it: a spare's slack beside the request, or, when the
        // ceiling has no room for that, an arena of its own.
        let mut charged = len;
        let extra = slack(&data, HEADROOM, len);
        if extra > 0 {
            if self.ledger.hold_more(extra) {
                charged += extra;
                lending.owed.bytes = Some(charged);
            } else {
                // Freed before the new one is allocated, so the two are never held at once. Only
                // a charge made between the trim and this one leaves no room for the slack.
                drop(std::mem::take(&mut data));
                data = arena(HEADROOM, len, None)?;
                at!(at_lend_step, LendStep::Allocated);
            }
        }
        at!(at_lend_step, LendStep::Backed);

        let mut lent = Box::new(Lent {
            tag: LENT_TAG,
            call: Arc::clone(self),
            data,
            headroom: HEADROOM,
            len,
            charged,
        });
        at!(at_lend_step, LendStep::Built);
        let ptr = lent.lent_ptr();
        Ok(ak_buffer {
            ptr,
            len,
            owner: Box::into_raw(lent) as *mut c_void,
        })
    }

    /// Charges a lend of `len` bytes, or records the send as waiting for room and tries again.
    ///
    /// Recorded, then tried again: a release between the refusal and the record owes this send
    /// nothing, and the second try is what sees it. Read against the end after the record, for
    /// the order `Ledger::stop_waiting` states.
    ///
    /// The record stands only on the refusal for the budget, whose wake-up the task raises. Any
    /// other way out takes it back, a panic included: a refused lend holds nothing, and a send
    /// that no lend is waiting for must not lower what every call may read.
    fn charge_or_wait(self: &Arc<Self>, len: usize) -> Result<(), ak_status> {
        if self.ledger.hold_bytes(len).is_ok() {
            return Ok(());
        }
        let waiting = Waiting { call: self };
        self.ledger.wait(&self.waiter, len);
        if !self.live() {
            return Err(ak_status::AK_STATUS_INVALID_STATE);
        }
        match self.ledger.hold_bytes(len) {
            Ok(()) => Ok(()),
            Err(refused) => {
                self.spawn_task();
                std::mem::forget(waiting);
                Err(refused)
            }
        }
    }

    /// Exchanges the buffer the host holds for one of `new_len` bytes, with the first `carried`
    /// bytes it wrote carried over: a lend of the new length and a return of the old that are one
    /// step for the ceiling, which sees the difference alone.
    ///
    /// A refusal leaves the buffer as it was, lent and charged, and the host holds it still; an
    /// overrun is the exception, as it is at the commit. A refusal for room records no wait and
    /// owes no wake-up: the host holds a buffer, and the wait is the lend's, made by a host that
    /// holds none. The call's one buffer, its window slot and its count against the ledger are the
    /// old buffer's and stay so, which is why none of the handshakes `lend` makes with the
    /// terminal is made again.
    ///
    /// A panic leaves the buffer as a refusal does. The buffer stays the host's until the exchange
    /// is done, so an unwind leaks it back to the host instead of freeing it: it is the same
    /// allocation at the same address, and `exchange` changes nothing of it until its last step.
    pub(crate) fn resize(
        self: &Arc<Self>,
        mut lent: Taken,
        new_len: usize,
        carried: usize,
    ) -> Result<ak_buffer, ak_status> {
        at!(at_resize_step, ResizeStep::Taken);
        // Before anything else: past an overrun, nothing the host passes alongside can be trusted.
        if carried > lent.len || !lent.intact() {
            // The box is gone whatever happens next, so the answer is CORRUPTED whatever a panic
            // in taking it back does: INTERNAL would invite a retry on an owner that no longer
            // exists.
            self.overrun(lent.into_box());
            return Err(ak_status::AK_STATUS_CORRUPTED);
        }
        let outcome = self.admits_buffer(new_len).and_then(|()| {
            at!(at_resize_step, ResizeStep::Admitted);
            self.exchange(&mut lent, new_len, carried)
        });
        if let Err(status) = outcome {
            return Err(lent.keep(status));
        }
        let ptr = lent.lent_ptr();
        Ok(ak_buffer {
            ptr,
            len: new_len,
            owner: lent.into_owner(),
        })
    }

    /// Replaces the lend's arena by one of `new_len` bytes, if the ceiling admits what that
    /// changes: an arena first, a spare of the channel's if one fits, the bytes carried into it,
    /// and then the charge, which is the last step that can refuse. The lend itself is not
    /// touched before the charge is made, and the step after it is plain assignments, so no
    /// refusal and no panic finds it half done. A panic in the charge is a refusal too:
    /// `Ledger::recharge` moves the charge in one step and contains what follows it, so the charge
    /// is either made, and the exchange with it, or not made, and the old charge stands.
    fn exchange(&self, lent: &mut Lent, new_len: usize, carried: usize) -> Result<(), ak_status> {
        if !self.ledger.has_room_to_recharge(lent.charged, new_len) {
            return Err(ak_status::AK_STATUS_BUDGET_BUSY);
        }
        let spares = &self.channel.spares;
        let mut data = arena(HEADROOM, new_len, Some(spares))?;
        at!(at_resize_step, ResizeStep::Allocated);
        #[cfg(feature = "test-hooks")]
        crate::hooks::run_before_charge();
        lent.carry_over(&mut data, new_len, carried);
        at!(at_resize_step, ResizeStep::Copied);
        let mut charged = new_len + slack(&data, HEADROOM, new_len);
        if !self.ledger.recharge(lent.charged, charged) {
            // The slack of a spare may have no room beside the request: an arena of its own is
            // charged the request alone.
            if charged == new_len {
                return Err(ak_status::AK_STATUS_BUDGET_BUSY);
            }
            drop(spares.returning(data));
            data = arena(HEADROOM, new_len, None)?;
            lent.carry_over(&mut data, new_len, carried);
            at!(at_resize_step, ResizeStep::Copied);
            charged = new_len;
            if !self.ledger.recharge(lent.charged, charged) {
                return Err(ak_status::AK_STATUS_BUDGET_BUSY);
            }
        }
        let left = lent.swap_arena(data, new_len, charged);
        // The exchange is made, and no panic in parking the old arena can unmake it: the host is
        // told it succeeded. Parked once the charge is off, which makes room for it beside the
        // others.
        guard_void(|| {
            at!(at_resize_step, ResizeStep::Exchanged);
            drop(spares.returning(left));
        });
        Ok(())
    }

    fn payload_returned(&self, returns_credit: bool) {
        if returns_credit {
            self.credits.add_permits(1);
        }
        self.debt.payloads.fetch_sub(1, Ordering::SeqCst);
        self.ledger.release();
        self.moved_on();
    }

    /// Commits a lent buffer as the next message. A refusal leaves the buffer lent and the host's,
    /// and so does a panic until the arena is taken to be the message: that is the point of no
    /// return, after which the host's buffer is gone and a panic answers CORRUPTED. Once the
    /// message is queued nothing undoes it, and a panic in what is left answers OK.
    pub(crate) fn commit(self: &Arc<Self>, lent: Taken) -> ak_status {
        let commands = match &self.requests {
            Requests::One(request) => return self.commit_one(request, Some(lent)),
            Requests::Stream(commands) => commands,
        };
        let Some(_queueing) = Queueing::enter(&self.sending) else {
            return lent.keep(ak_status::AK_STATUS_INVALID_STATE);
        };
        #[cfg(feature = "test-hooks")]
        crate::hooks::run_before_queueing();

        if !self.accepts_work() {
            return lent.keep(ak_status::AK_STATUS_INVALID_STATE);
        }

        let Ok(slot) = commands.try_reserve() else {
            return lent.keep(ak_status::AK_STATUS_INVALID_STATE);
        };
        at!(at_send_step, SendStep::Admitted);
        let charged = lent.charged;
        let Ok(message) = catch_unwind(AssertUnwindSafe(|| self.frame(lent))) else {
            self.forfeit(charged);
            return ak_status::AK_STATUS_CORRUPTED;
        };
        // Counted before the message is queued: its WRITE_DONE may let the host ask for a buffer
        // from inside the callback, which the one buffer a call holds would refuse.
        self.debt.buffers.fetch_sub(1, Ordering::SeqCst);
        slot.send(Command::Send { message, charged });
        guard_void(|| {
            at!(at_send_step, SendStep::Queued);
        });
        guard_void(|| self.moved_on());
        ak_status::AK_STATUS_OK
    }

    /// The message a lent buffer's arena becomes, framed in place. The host's buffer is consumed:
    /// a panic leaves nothing of it.
    fn frame(&self, lent: Taken) -> FramedMessage {
        let lent = lent.into_box();
        at!(at_send_step, SendStep::Framing);
        self.channel
            .spares
            .framed(lent.data)
            .expect("lent with its prefix ahead")
    }

    /// Queues an empty message, which no buffer carries.
    ///
    /// It takes a slot of the window as a lend does, given back at its WRITE_DONE, and is counted
    /// like one so a shutdown waits for that acquittal: the writer gives back the count with the
    /// message's bytes, of which there are none.
    pub(crate) fn commit_empty(self: &Arc<Self>) -> ak_status {
        let commands = match &self.requests {
            Requests::One(request) => return self.commit_one(request, None),
            Requests::Stream(commands) => commands,
        };
        let Some(_queueing) = Queueing::enter(&self.sending) else {
            return ak_status::AK_STATUS_INVALID_STATE;
        };
        if !self.accepts_work() {
            return ak_status::AK_STATUS_INVALID_STATE;
        }
        let Ok(window) = self.window.try_acquire() else {
            return ak_status::AK_STATUS_SLOT_BUSY;
        };
        let Ok(slot) = commands.try_reserve() else {
            return ak_status::AK_STATUS_INVALID_STATE;
        };
        self.ledger.hold();
        window.forget();
        slot.send(Command::Send {
            message: FramedMessage::empty(),
            charged: 0,
        });
        ak_status::AK_STATUS_OK
    }

    /// Commits a call's one request, which also ends its sending: SendMessage then EndSend, with
    /// nothing of the call between them, as the state they share is held here. Settled at once, as
    /// a WRITE_DONE settles a stream's send, and with no acquittal: the host gave up its only
    /// buffer, and its slot and its bytes go back now. The task is spawned here unless something
    /// needed it earlier, in which case it takes the request from the slot.
    ///
    /// The point of no return is as the stream's commit has it: the buffer is the host's until the
    /// arena is taken to be the message, and the request, once given, is made whatever a panic in
    /// its accounting does.
    fn commit_one(self: &Arc<Self>, request: &OneRequest, lent: Option<Taken>) -> ak_status {
        let refused = |lent: Option<Taken>| match lent {
            Some(lent) => lent.keep(ak_status::AK_STATUS_INVALID_STATE),
            None => ak_status::AK_STATUS_INVALID_STATE,
        };
        if !self.accepts_work() {
            return refused(lent);
        }
        at!(at_send_step, SendStep::Admitted);
        if self
            .sending
            .compare_exchange(0, SENDING_ENDED, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return refused(lent);
        }

        let charged = lent.as_ref().map(|lent| lent.charged);
        let mut lent = lent;
        let given = catch_unwind(AssertUnwindSafe(|| {
            at!(at_send_step, SendStep::Ending);
            request.give(|| match lent.take() {
                Some(lent) => self.frame(lent),
                None => FramedMessage::empty(),
            })
        }));
        let given = match given {
            Ok(given) => given,
            Err(panic) => {
                // The arena was taken and the request is not given: the buffer is gone.
                if let (Some(charged), None) = (charged, &lent) {
                    self.forfeit(charged);
                    return ak_status::AK_STATUS_CORRUPTED;
                }
                // Nothing was taken or given: the sending the claim above ended is open again.
                self.sending.store(0, Ordering::Release);
                resume_unwind(panic);
            }
        };
        if !given {
            return refused(lent);
        }

        // The request is given: each part of what is left is made whatever another does.
        guard_void(|| {
            at!(at_send_step, SendStep::Queued);
        });
        if let Some(charged) = charged {
            self.repay(Owed::lent(charged));
        }
        // A task that is not spawned is a call that never ends: the shutdown is what is left.
        if catch_unwind(AssertUnwindSafe(|| {
            at!(at_send_step, SendStep::Spawning);
            self.spawn_task();
        }))
        .is_err()
        {
            self.shut_down();
        }
        ak_status::AK_STATUS_OK
    }

    /// Takes back a buffer the host gives back, which a panic cannot refuse: the host has nothing
    /// to retry, so its debt is paid whatever a panic meets. A panic while the bytes after the
    /// buffer are read leaves it unknown whether the memory is sound, which an overrun answers.
    pub(crate) fn return_buffer(&self, lent: Taken) {
        let intact = catch_unwind(AssertUnwindSafe(|| {
            at!(at_return_step, ReturnStep::Taken);
            lent.intact()
        }));
        let lent = lent.into_box();
        if let Ok(true) = intact {
            self.repay(Owed::lent(lent.charged));
        } else {
            self.overrun(lent);
        }
    }

    /// Takes back a buffer the host overran, without freeing it, and shuts the runtime down: the
    /// memory around it may be corrupted, and nothing this library does in it can be trusted.
    ///
    /// Never unwinds: the buffer is gone from the host whatever happens here, so the answer is
    /// CORRUPTED whatever happens here.
    pub(crate) fn overrun(&self, lent: Box<Lent>) {
        let (_, charged) = lent.abandon();
        self.forfeit(charged);
    }

    /// Ends a lend the host has no part in any more and cannot give back, and shuts the runtime
    /// down, since what took it is not to be trusted. The debt is paid, so the shutdown can
    /// complete.
    fn forfeit(&self, charged: usize) {
        self.repay(Owed::lent(charged));
        self.shut_down();
    }

    fn shut_down(&self) {
        guard_void(|| {
            if let Some(runtime) = crate::tables::runtimes().get(self.channel.runtime) {
                crate::lifecycle::begin_shutdown(&runtime);
            }
        });
    }

    /// Pays what a lend owes the call and the ledger: its one buffer, the bytes and the count it
    /// charged, its slot of the window, and the wake-ups a payment owes.
    ///
    /// Each part is made whatever another does, the counts before the wake-ups they owe: a part
    /// left unpaid is a runtime that never quiesces.
    fn repay(&self, owed: Owed) {
        guard_void(|| {
            at!(at_repay_step, RepayStep::Begun);
        });
        guard_void(|| {
            self.debt.buffers.fetch_sub(1, Ordering::SeqCst);
            at!(at_repay_step, RepayStep::Counted);
        });
        guard_void(|| {
            if let Some(charged) = owed.bytes {
                self.ledger.release_bytes(charged);
            }
            at!(at_repay_step, RepayStep::Released);
        });
        guard_void(|| {
            if owed.slot {
                self.window.add_permits(1);
            }
            at!(at_repay_step, RepayStep::Permitted);
        });
        guard_void(|| self.moved_on());
    }

    pub(crate) fn end_send(&self) -> ak_status {
        // A call that sends one request ends its sending with it, and one with no request is not
        // a call gRPC has: a host that wants none cancels.
        let Requests::Stream(commands) = &self.requests else {
            return ak_status::AK_STATUS_INVALID_STATE;
        };
        if !self.live() {
            return ak_status::AK_STATUS_INVALID_STATE;
        }
        let Ok(slot) = commands.try_reserve() else {
            return ak_status::AK_STATUS_INVALID_STATE;
        };
        // From no send being counted and the sending still open. An end already queued is a
        // refusal, and the slot goes back unused with it; a send still counted is waited for,
        // briefly: its downcall queues without waiting on anything, so it returns.
        let mut seen = self.sending.load(Ordering::Acquire);
        loop {
            if seen & SENDING_ENDED != 0 {
                return ak_status::AK_STATUS_INVALID_STATE;
            }
            if seen != 0 {
                std::thread::yield_now();
                seen = self.sending.load(Ordering::Acquire);
                continue;
            }
            match self.sending.compare_exchange_weak(
                0,
                SENDING_ENDED,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(now) => seen = now,
            }
        }
        slot.send(Command::EndSend);
        ak_status::AK_STATUS_OK
    }

    fn in_callback(&self, emit: impl FnOnce()) {
        self.debt.callbacks.fetch_add(1, Ordering::AcqRel);
        emit();
        self.debt.callbacks.fetch_sub(1, Ordering::SeqCst);
        // Quiet is what `finished` waits for, and settled implies it. This is also what announces
        // a debt paid inside the callback, before the terminal, which `moved_on` saw no reason to,
        // and what reclaims a call whose host paid everything there.
        if self.debt.quiet() {
            self.announce();
            self.reclaim_if_settled();
        }
    }

    /// Announces and reclaims a settled call, and only a settled one: an announcement for a debt
    /// still owed wakes whoever waits on the call for nothing, and a host that gives back a
    /// callback's payloads one by one would pay that wake-up for each.
    ///
    /// The counters' decrements and the reads in `settled` are sequentially consistent, so of two
    /// last debts paid at once, the one later in that order reads the other paid, and announces
    /// and reclaims.
    fn moved_on(&self) {
        if self.debt.settled() {
            self.announce();
            self.reclaim_if_settled();
        }
    }

    /// Reclaims a settled call on the thread that paid its last debt: a downcall that pays it
    /// must not pay a wake-up of the channel's thread besides.
    fn reclaim_if_settled(&self) {
        if self.debt.settled() && !self.reclaimed.swap(true, Ordering::AcqRel) {
            crate::lifecycle::call_settled(self.handle);
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
