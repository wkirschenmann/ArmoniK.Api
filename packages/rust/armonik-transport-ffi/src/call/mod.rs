use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use armonik_transport::grpc::CallControl;
use bytes::Bytes;
use tokio::sync::{mpsc, watch, Semaphore};

use crate::abi::{ak_buffer, ak_call_debt, ak_handle, ak_status};
use crate::channel::AkChannel;
use crate::host::{Host, HostPtr};
use crate::ledger::{Ledger, Waiter};

mod actor;
mod lent;
mod start;
mod turn;

use lent::{arena, LENT_TAG};
pub(crate) use lent::{keep, take_lent, take_payload, Lent};
pub(crate) use start::start_on;
use turn::ReadTurn;

pub(crate) struct CallServices<'a> {
    pub(crate) host: &'a Arc<Host>,
    pub(crate) ledger: &'a Arc<Ledger>,
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
    commands: mpsc::Sender<Command>,
    window: Semaphore,
    credits: Semaphore,
    debt: Debt,
    cancelled: AtomicBool,
    progress: watch::Sender<u64>,
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

    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.announce();
        self.control.cancel();
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
        if self.ledger.hold_bytes(len).is_err() {
            // Recorded, then tried again: a release between the refusal and the record owes this
            // send nothing, and the second try is what sees it. Read against the end after the
            // record, for the order `Ledger::stop_waiting` states.
            self.ledger.wait(&self.waiter, len);
            if !self.live() {
                self.ledger.stop_waiting(&self.waiter);
                return Err(ak_status::AK_STATUS_INVALID_STATE);
            }
            self.ledger.hold_bytes(len)?;
        }
        // Served: reads are no longer held back for it. Only a lend of this call records a wait,
        // and one lend runs at a time, so a wait this reads as absent is absent.
        if self.waiter.is_waiting() {
            self.ledger.stop_waiting(&self.waiter);
        }

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
        self.debt.buffers.fetch_sub(1, Ordering::SeqCst);
        self.ledger.release_bytes(len);
        self.moved_on();
    }

    fn handed_over(&self) {
        self.debt.buffers.fetch_sub(1, Ordering::SeqCst);
        self.moved_on();
    }

    fn payload_returned(&self, returns_credit: bool) {
        if returns_credit {
            self.credits.add_permits(1);
        }
        self.debt.payloads.fetch_sub(1, Ordering::SeqCst);
        self.ledger.release();
        self.moved_on();
    }

    pub(crate) fn commit(&self, lent: Box<Lent>) -> ak_status {
        let Some(_queueing) = Queueing::enter(&self.sending) else {
            return keep(lent, ak_status::AK_STATUS_INVALID_STATE);
        };
        #[cfg(feature = "test-hooks")]
        crate::hooks::run_before_queueing();

        if !self.accepts_work() {
            return keep(lent, ak_status::AK_STATUS_INVALID_STATE);
        }

        let Ok(slot) = self.commands.try_reserve() else {
            return keep(lent, ak_status::AK_STATUS_INVALID_STATE);
        };
        self.handed_over();
        slot.send(Command::Send(Bytes::from(lent.data)));
        ak_status::AK_STATUS_OK
    }

    /// Queues an empty message, which no buffer carries.
    ///
    /// It takes a slot of the window as a lend does, given back at its WRITE_DONE, and is counted
    /// like one so a shutdown waits for that acquittal: the writer gives back the count with the
    /// message's bytes, of which there are none.
    pub(crate) fn commit_empty(&self) -> ak_status {
        let Some(_queueing) = Queueing::enter(&self.sending) else {
            return ak_status::AK_STATUS_INVALID_STATE;
        };
        if !self.accepts_work() {
            return ak_status::AK_STATUS_INVALID_STATE;
        }
        let Ok(window) = self.window.try_acquire() else {
            return ak_status::AK_STATUS_SLOT_BUSY;
        };
        let Ok(slot) = self.commands.try_reserve() else {
            return ak_status::AK_STATUS_INVALID_STATE;
        };
        self.ledger.hold();
        window.forget();
        slot.send(Command::Send(Bytes::new()));
        ak_status::AK_STATUS_OK
    }

    #[allow(clippy::boxed_local)]
    pub(crate) fn give_back(&self, lent: Box<Lent>) {
        self.took_back(lent.data.len());
        self.window.add_permits(1);
    }

    pub(crate) fn end_send(&self) -> ak_status {
        if !self.live() {
            return ak_status::AK_STATUS_INVALID_STATE;
        }
        let Ok(slot) = self.commands.try_reserve() else {
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
        // Quiet is what `finished` waits for; settled, which the reclaim waits for, implies it. This
        // is also what announces a debt paid inside the callback, before the terminal, which
        // `moved_on` saw no reason to.
        if self.debt.quiet() {
            self.announce();
        }
    }

    /// Wakes the reclaim once the call is settled, and not before: a wake-up that finds a debt
    /// still owed costs the channel's thread a turn for nothing, and a host that gives back a
    /// callback's payloads one by one would pay it for each.
    ///
    /// The counters' decrements and the reads in `settled` are sequentially consistent, so of two
    /// last debts paid at once, the one later in that order reads the other paid, and announces.
    fn moved_on(&self) {
        if self.debt.settled() {
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
