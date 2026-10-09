use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use armonik_transport::grpc::{Charge, CompressionBudget};
use tokio::sync::{watch, Notify};

use crate::abi::{ak_memory_usage, ak_status};
use crate::held::Held;
use crate::spares::Spares;

/// The most this library lends for one message, whatever the host asked for.
///
/// Two bounds meeting: the gRPC length prefix is four bytes, and no allocation exceeds
/// `isize::MAX`, which on a 32-bit target is the smaller of the two.
///
/// It caps the ceiling rather than standing beside it. A ceiling above it would be a budget no
/// single lend could ever draw on, and telling the two apart would mean two reasons for one
/// status - `AK_STATUS_MESSAGE_TOO_LARGE` says `len` is past the ceiling, and this is what makes
/// that one sentence true. What it costs is a runtime-wide emission budget of four gigabytes,
/// which is four gigabytes of serialization arenas out at once.
const LARGEST_LENDABLE: u64 = if (u32::MAX as u64) < (isize::MAX as u64) {
    u32::MAX as u64
} else {
    isize::MAX as u64
};

/// What a call's send refused for room waits on, and the wake-up a release owes it.
#[derive(Default)]
pub(crate) struct Waiter {
    /// The length the send asked for, zero while none waits.
    len: AtomicUsize,
    owed: AtomicBool,
    notify: Notify,
}

impl Waiter {
    /// Completes once a release has owed this call a wake-up since the last one was taken.
    pub(crate) async fn notified(&self) {
        self.notify.notified().await;
    }

    pub(crate) fn take_owed(&self) -> bool {
        self.owed.swap(false, Ordering::AcqRel)
    }

    pub(crate) fn is_waiting(&self) -> bool {
        self.len.load(Ordering::Acquire) != 0
    }
}

/// One count of bytes over what is sent and what is received, and two thresholds over it.
///
/// The first is where work waits: a call stops reading, and a lend is refused with
/// AK_STATUS_BUDGET_BUSY. The second is where the engine stops: a decoded message that would pass
/// it ends its call. Calls admitted to read below the first may pass it together, by a message
/// each, and the second is what bounds them.
pub(crate) struct Ledger {
    outstanding: AtomicU64,
    bytes: AtomicU64,
    ceiling: u64,
    hard_ceiling: u64,
    changed: watch::Sender<u64>,
    /// Moved by every release of bytes and every send that stops waiting: what a read held back
    /// waits on.
    room: watch::Sender<u64>,
    waiting: Mutex<Vec<Arc<Waiter>>>,
    /// The bytes the channels keep in spare arenas: under the first threshold with the charges,
    /// and given up for a charge that needs their room. Not in `bytes`, which is what the host
    /// owes and is told of.
    spare: AtomicU64,
    /// The channels' spares, which a charge that needs their room empties.
    spares: Mutex<Vec<Weak<Spares>>>,
}

impl Ledger {
    /// A second threshold below the first is refused: it would end calls the first lets read.
    pub(crate) fn new(ceiling: u64, hard_ceiling: u64) -> Result<Self, ak_status> {
        let ledger = Self {
            outstanding: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            ceiling,
            hard_ceiling,
            changed: watch::channel(0).0,
            room: watch::channel(0).0,
            waiting: Mutex::new(Vec::new()),
            spare: AtomicU64::new(0),
            spares: Mutex::new(Vec::new()),
        };
        if hard_ceiling != 0 && hard_ceiling < ledger.limit() {
            return Err(ak_status::AK_STATUS_INVALID_ARG);
        }
        Ok(ledger)
    }

    pub(crate) fn usage(&self) -> ak_memory_usage {
        ak_memory_usage {
            bytes_used: self.bytes.load(Ordering::Acquire),
            ceiling: self.limit(),
        }
    }

    /// The ceiling in force, which is never zero and never above what one lend can be.
    ///
    /// A host that asks for no ceiling gets this library's own; a host that asks for more than it
    /// gets this library's own. Reported as such by `ak_runtime_memory_usage`, because a
    /// published number a host divides by should be the one being enforced.
    fn limit(&self) -> u64 {
        let asked = if self.ceiling == 0 {
            u64::MAX
        } else {
            self.ceiling
        };
        asked.min(LARGEST_LENDABLE)
    }

    /// The second threshold in force: as configured, or a quarter above the first.
    fn hard_limit(&self) -> u64 {
        if self.hard_ceiling == 0 {
            let limit = self.limit();
            limit.saturating_add(limit / 4)
        } else {
            self.hard_ceiling
        }
    }

    fn waiting(&self) -> Held<MutexGuard<'_, Vec<Arc<Waiter>>>> {
        Held::new(self.waiting.lock().unwrap_or_else(PoisonError::into_inner))
    }

    // Sequentially consistent here and in `empty`: `CallState::fill` counts a lend and then reads
    // its call's terminal, and `shutting_down` reads every terminal through `Debt::quiet` and
    // then the count, so one of the two sees the other's write.
    pub(crate) fn hold(&self) {
        self.outstanding.fetch_add(1, Ordering::SeqCst);
    }

    pub(crate) fn release(&self) {
        if self.outstanding.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.changed.send_modify(|version| *version += 1);
        }
    }

    pub(crate) fn could_ever_fit(&self, len: usize) -> Result<(), ak_status> {
        if len as u64 <= self.limit() {
            Ok(())
        } else {
            Err(ak_status::AK_STATUS_MESSAGE_TOO_LARGE)
        }
    }

    /// Charges a lend against the ceiling, counted before it is charged.
    ///
    /// The count goes up first and comes down last, on both sides, so it is conservative in one
    /// direction only: it can name a lend that is not charged yet, and never a charge nothing
    /// counts. That is the direction `empty` needs. Counted after the bytes, a thread preempted
    /// between the two would leave the ledger holding bytes that nothing was counting - and
    /// `empty` is what decides whether the shutdown owes RESOURCES_RELEASED and whether it waits
    /// for the host to give anything back. Answered wrongly there, the runtime reports QUIESCENT
    /// with a buffer still lent, which is the one thing that state is promised not to mean.
    ///
    /// A panic leaves nothing charged and nothing counted: `add_bytes` charges nothing when it
    /// panics, and the count goes back with the panic.
    pub(crate) fn hold_bytes(&self, len: usize) -> Result<(), ak_status> {
        if self.counted(|| self.add_bytes(len)) {
            Ok(())
        } else {
            Err(ak_status::AK_STATUS_BUDGET_BUSY)
        }
    }

    /// Counts a charge, then makes it with `charge`, which says whether it was made. A charge
    /// refused, or one that panics before it moves the bytes, gives its count back.
    fn counted(&self, charge: impl FnOnce() -> bool) -> bool {
        self.hold();
        match catch_unwind(AssertUnwindSafe(charge)) {
            Ok(true) => true,
            Ok(false) => {
                self.release();
                false
            }
            Err(panic) => {
                self.release();
                resume_unwind(panic)
            }
        }
    }

    /// Charges `len` more to a lend already counted, unless that would pass the first threshold:
    /// the slack of the spare arena it took.
    pub(crate) fn hold_more(&self, len: usize) -> bool {
        self.add_bytes(len)
    }

    /// Charges a copy the engine holds, unless that would pass the first threshold. It is bytes
    /// only: the host owes nothing for it, so it is not in the count a shutdown waits on.
    pub(crate) fn hold_copy(&self, len: usize) -> bool {
        self.add_bytes(len)
    }

    /// Gives a copy's bytes back, and owes the sends that wait their wake-up.
    pub(crate) fn release_copy(&self, len: usize) {
        self.bytes.fetch_sub(len as u64, Ordering::AcqRel);
        if len > 0 {
            self.room_made();
        }
    }

    /// Whether a lend charged `old` bytes could be charged `new` instead right now, which is the
    /// ceiling's answer without the step: a resize that finds none allocates nothing.
    pub(crate) fn has_room_to_recharge(&self, old: usize, new: usize) -> bool {
        new <= old
            || self
                .bytes
                .load(Ordering::SeqCst)
                .checked_add((new - old) as u64)
                .is_some_and(|wanted| wanted <= self.limit())
    }

    /// Charges a lend already counted `new` bytes where it was charged `old`, in one step: the
    /// count moves by the difference alone, so it is never both and never neither, and a growth
    /// is admitted against the first threshold as a lend of that difference is.
    ///
    /// A panic reaches the caller only before the charge moves, which leaves it `old`: what
    /// follows the step is contained.
    pub(crate) fn recharge(&self, old: usize, new: usize) -> bool {
        if new > old {
            return self.add_bytes(new - old);
        }
        if new < old {
            at!(at_charge_step, ChargeStep::Begun);
            self.bytes.fetch_sub((old - new) as u64, Ordering::AcqRel);
            crate::guard_void(|| {
                at!(at_charge_step, ChargeStep::Moved);
                self.room_made();
            });
        }
        true
    }

    fn add_bytes(&self, len: usize) -> bool {
        self.add_bytes_up_to(len, self.limit())
    }

    /// Charges `len` unless that would pass `limit`: the first threshold for a lend, the second
    /// for a received message.
    ///
    /// Sequentially consistent, as `keep_spare` and the trim after it are: a charge adds to its
    /// count and then reads the spares', a spare is kept and then the trim reads the charges, so
    /// the second of the two sees both and gives the spares up.
    ///
    /// A panic reaches the caller only before the count moves, which leaves nothing charged: what
    /// follows the step is contained.
    fn add_bytes_up_to(&self, len: usize, limit: u64) -> bool {
        at!(at_charge_step, ChargeStep::Begun);
        let mut seen = self.bytes.load(Ordering::SeqCst);
        loop {
            let Some(wanted) = seen
                .checked_add(len as u64)
                .filter(|wanted| *wanted <= limit)
            else {
                return false;
            };
            match self
                .bytes
                .compare_exchange_weak(seen, wanted, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => {
                    crate::guard_void(|| {
                        at!(at_charge_step, ChargeStep::Moved);
                        self.trim_spares();
                    });
                    return true;
                }
                Err(current) => seen = current,
            }
        }
    }

    /// Counts a spare arena of `capacity` bytes, if the first threshold has room for it beside
    /// what is charged and what is already kept. Correct only with `trim_spares` after the arena
    /// is kept: a charge made meanwhile may not have seen it.
    pub(crate) fn keep_spare(&self, capacity: usize) -> bool {
        let mut seen = self.spare.load(Ordering::SeqCst);
        loop {
            let kept = seen.saturating_add(capacity as u64);
            if self.bytes.load(Ordering::SeqCst).saturating_add(kept) > self.limit() {
                return false;
            }
            match self
                .spare
                .compare_exchange_weak(seen, kept, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => return true,
                Err(current) => seen = current,
            }
        }
    }

    pub(crate) fn drop_spare(&self, capacity: usize) {
        self.spare.fetch_sub(capacity as u64, Ordering::SeqCst);
    }

    /// Takes `spares` in among those a charge can empty, and lets go of the channels gone.
    pub(crate) fn register(&self, spares: &Arc<Spares>) {
        let mut registered = Held::new(self.spares.lock().unwrap_or_else(PoisonError::into_inner));
        registered.retain(|spares| spares.strong_count() > 0);
        registered.push(Arc::downgrade(spares));
    }

    fn over_with_spares(&self) -> bool {
        self.bytes
            .load(Ordering::SeqCst)
            .saturating_add(self.spare.load(Ordering::SeqCst))
            > self.limit()
    }

    /// Gives up spares, a channel's at a time, while they and the charges pass the first
    /// threshold: a charge is never refused for them, and they leave it the room. A spare kept
    /// while a charge was being made, which that charge could not see, calls it too.
    pub(crate) fn trim_spares(&self) {
        if self.spare.load(Ordering::SeqCst) == 0 || !self.over_with_spares() {
            return;
        }
        let registered = Held::new(self.spares.lock().unwrap_or_else(PoisonError::into_inner));
        for spares in registered.iter().filter_map(Weak::upgrade) {
            spares.clear();
            if !self.over_with_spares() {
                break;
            }
        }
    }

    pub(crate) fn release_bytes(&self, len: usize) {
        self.bytes.fetch_sub(len as u64, Ordering::AcqRel);
        // The count before the wake-ups: a panic in them leaves what the shutdown waits on paid.
        self.release();
        if len > 0 {
            self.room_made();
        }
    }

    /// Owes every waiting send its wake-up, all of them: waking one would lose the wake-up when
    /// that one does not try again.
    fn room_made(&self) {
        for waiter in self.waiting().iter() {
            waiter.owed.store(true, Ordering::Release);
            waiter.notify.notify_one();
        }
        self.room.send_modify(|version| *version += 1);
    }

    /// Charges a decoded message, unless it would take the count past the second threshold.
    /// Counted like a lend, so a shutdown waits for it.
    ///
    /// A panic reaches the caller only before the charge moves, with nothing charged and nothing
    /// counted: what follows the move is contained, and the message is held.
    pub(crate) fn hold_received(self: &Arc<Self>, len: usize) -> Option<Received> {
        self.counted(|| self.add_bytes_up_to(len, self.hard_limit()))
            .then(|| Received {
                ledger: Arc::clone(self),
                len,
            })
    }

    /// A send refused for room waits on `len`, which holds reads back until it is served.
    pub(crate) fn wait(&self, waiter: &Arc<Waiter>, len: usize) {
        let mut waiting = self.waiting();
        if waiter.len.swap(len, Ordering::AcqRel) == 0 {
            waiting.push(Arc::clone(waiter));
        }
    }

    /// The send is served or its call is over: reads are no longer held back for it.
    ///
    /// Always under the lock, which is what orders it against `wait`: a call ending while its
    /// send is refused either finds the record here or is seen ending by the send.
    pub(crate) fn stop_waiting(&self, waiter: &Waiter) {
        {
            let mut waiting = self.waiting();
            if waiter.len.swap(0, Ordering::AcqRel) == 0 {
                return;
            }
            waiting.retain(|other| !std::ptr::eq(Arc::as_ptr(other), waiter));
        }
        waiter.owed.store(false, Ordering::Release);
        self.room.send_modify(|version| *version += 1);
    }

    /// Reads stop at the first threshold, lowered by the largest length a refused send waits on,
    /// so that send is served before new reads. The length and not the charge: a length is never
    /// past the first threshold, so the lowered one is never below zero.
    fn admits_read(&self) -> bool {
        let lowered = self
            .waiting()
            .iter()
            .map(|waiter| waiter.len.load(Ordering::Acquire) as u64)
            .max()
            .unwrap_or(0);
        self.bytes.load(Ordering::Acquire).saturating_add(lowered) < self.limit()
    }

    /// The first step of a read, where the decision is taken: completes once a read is admitted.
    pub(crate) async fn read_admitted(&self) {
        let _ = self.room.subscribe().wait_for(|_| self.admits_read()).await;
    }

    pub(crate) fn empty(&self) -> bool {
        self.outstanding.load(Ordering::SeqCst) == 0
    }

    pub(crate) async fn drained(&self) {
        let _ = self.changed.subscribe().wait_for(|_| self.empty()).await;
    }
}

/// A received message's charge, given back when the host consumes it or when its call drops it
/// undelivered.
pub(crate) struct Received {
    ledger: Arc<Ledger>,
    len: usize,
}

impl Drop for Received {
    fn drop(&mut self) {
        self.ledger.release_bytes(self.len);
    }
}

/// What the engine's compression asks of the ceiling for the copies it makes. A copy is charged as
/// a lend is, against the first threshold, and never waits for room: the engine sends the message
/// as it is instead. The engine holds a copy, not the host, so a shutdown does not wait on it as it
/// does on a lend.
pub(crate) struct CopyBudget(pub(crate) Arc<Ledger>);

impl std::fmt::Debug for CopyBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CopyBudget").finish_non_exhaustive()
    }
}

impl CompressionBudget for CopyBudget {
    fn charge(&self, bytes: usize) -> Option<Charge> {
        #[cfg(feature = "test-hooks")]
        crate::hooks::run_before_copy_charge();
        if !self.0.hold_copy(bytes) {
            return None;
        }
        Some(Box::new(Copied {
            ledger: Arc::clone(&self.0),
            len: bytes,
        }))
    }
}

/// A compressed copy's charge, given back with the message that holds the copy.
struct Copied {
    ledger: Arc<Ledger>,
    len: usize,
}

impl Drop for Copied {
    fn drop(&mut self) {
        self.ledger.release_copy(self.len);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A refused charge leaves the ledger as it found it, count included.
    ///
    /// The count is raised before the bytes, so the refusal has an undo - and without it a host
    /// that met the ceiling once would leave the ledger never empty again, which is a runtime that
    /// never reaches QUIESCENT and an `ak_runtime_destroy` refused for the life of the process.
    #[test]
    fn a_charge_the_ceiling_refuses_leaves_nothing_counted() {
        let ledger = Ledger::new(64, 0).expect("a valid ledger");

        assert!(ledger.empty());
        assert_eq!(ledger.hold_bytes(65), Err(ak_status::AK_STATUS_BUDGET_BUSY));
        assert!(ledger.empty(), "the refusal gave its count back");
        assert_eq!(ledger.usage().bytes_used, 0);
    }

    /// A copy is bytes against the first threshold and no count: the engine holds it, so a shutdown
    /// that waits for what the host owes does not wait for it.
    #[test]
    fn a_copy_is_charged_without_a_count() {
        let ledger = Ledger::new(64, 0).expect("a valid ledger");

        assert!(ledger.hold_copy(40));
        assert!(ledger.empty());
        assert_eq!(ledger.usage().bytes_used, 40);
        assert!(!ledger.hold_copy(25), "past the ceiling");
        assert_eq!(ledger.usage().bytes_used, 40);

        ledger.release_copy(40);
        assert_eq!(ledger.usage().bytes_used, 0);
    }

    /// And what it counts is raised before the bytes it charges, which is the order `empty` reads.
    #[test]
    fn a_charge_is_counted_while_it_is_held() {
        let ledger = Ledger::new(64, 0).expect("a valid ledger");

        assert_eq!(ledger.hold_bytes(40), Ok(()));
        assert!(!ledger.empty());
        assert_eq!(ledger.usage().bytes_used, 40);

        ledger.release_bytes(40);
        assert!(ledger.empty());
        assert_eq!(ledger.usage().bytes_used, 0);
    }

    /// A lend recharged moves the count by the difference alone: a growth that the ceiling holds
    /// whole is admitted though the two sizes together would pass it, one it does not hold leaves
    /// the charge as it was, and a shrink gives the difference back.
    #[test]
    fn a_recharge_is_admitted_by_its_difference() {
        let ledger = Ledger::new(64, 0).expect("a valid ledger");
        assert_eq!(ledger.hold_bytes(40), Ok(()));

        assert!(ledger.has_room_to_recharge(40, 64));
        assert!(ledger.recharge(40, 64), "40 and 64 are never held at once");
        assert_eq!(ledger.usage().bytes_used, 64);

        assert!(!ledger.has_room_to_recharge(64, 65));
        assert!(!ledger.recharge(64, 65));
        assert_eq!(ledger.usage().bytes_used, 64, "the refusal charged nothing");

        assert!(ledger.recharge(64, 8));
        assert_eq!(ledger.usage().bytes_used, 8);
        assert!(!ledger.empty(), "the lend is still counted");

        ledger.release_bytes(8);
        assert!(ledger.empty());
        assert_eq!(ledger.usage().bytes_used, 0);
    }

    /// A shrink gives bytes back, so it owes the sends waiting for room their wake-up.
    #[test]
    fn a_shrink_wakes_a_waiting_send() {
        let ledger = Ledger::new(64, 0).expect("a valid ledger");
        let waiter = Arc::new(Waiter::default());
        assert_eq!(ledger.hold_bytes(60), Ok(()));
        ledger.wait(&waiter, 30);

        assert!(ledger.recharge(60, 20));
        assert!(waiter.take_owed());
        ledger.release_bytes(20);
    }

    /// The second threshold is a quarter above the first unless set, and never below it.
    #[test]
    fn the_second_threshold_defaults_above_the_first() {
        assert_eq!(
            Ledger::new(64, 32).err(),
            Some(ak_status::AK_STATUS_INVALID_ARG),
            "a second threshold below the first would end calls the first lets read"
        );
        assert_eq!(Ledger::new(64, 0).expect("a valid ledger").hard_limit(), 80);
        assert_eq!(
            Ledger::new(64, 100).expect("a valid ledger").hard_limit(),
            100
        );
    }

    /// A received message may take the count past the first threshold and not past the second,
    /// and one refused there leaves nothing counted.
    #[test]
    fn a_received_message_is_charged_up_to_the_second_threshold() {
        let ledger = Arc::new(Ledger::new(64, 100).expect("a valid ledger"));

        let first = ledger
            .hold_received(90)
            .expect("below the second threshold");
        assert_eq!(ledger.usage().bytes_used, 90);
        assert!(
            ledger.hold_received(11).is_none(),
            "past the second threshold"
        );
        assert_eq!(ledger.usage().bytes_used, 90, "the refusal charged nothing");

        drop(first);
        assert!(ledger.empty());
        assert_eq!(ledger.usage().bytes_used, 0);
    }

    /// A refused send lowers the threshold reads stop at by its length, until it stops waiting.
    #[test]
    fn a_waiting_send_holds_reads_back() {
        let ledger = Arc::new(Ledger::new(64, 0).expect("a valid ledger"));
        let waiter = Arc::new(Waiter::default());
        let held = ledger.hold_received(30).expect("room");
        assert!(ledger.admits_read());

        ledger.wait(&waiter, 40);
        assert!(
            !ledger.admits_read(),
            "30 held and 40 waiting reach the ceiling"
        );

        ledger.stop_waiting(&waiter);
        assert!(ledger.admits_read());
        drop(held);
    }

    /// A release that gives bytes back owes every waiting send its wake-up; one that stopped
    /// waiting is owed nothing.
    #[test]
    fn a_release_wakes_every_waiting_send() {
        let ledger = Arc::new(Ledger::new(64, 0).expect("a valid ledger"));
        let (one, other, served) = (
            Arc::new(Waiter::default()),
            Arc::new(Waiter::default()),
            Arc::new(Waiter::default()),
        );
        let held = ledger.hold_received(60).expect("room");
        ledger.wait(&one, 10);
        ledger.wait(&other, 20);
        ledger.wait(&served, 30);
        ledger.stop_waiting(&served);

        drop(held);

        assert!(one.take_owed());
        assert!(other.take_owed());
        assert!(!served.take_owed());
        assert!(!one.take_owed(), "a wake-up is taken once");
    }
}
