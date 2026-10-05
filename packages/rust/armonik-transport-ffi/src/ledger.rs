use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

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
    pub(crate) fn hold_bytes(&self, len: usize) -> Result<(), ak_status> {
        self.hold();
        if self.add_bytes(len) {
            Ok(())
        } else {
            self.release();
            Err(ak_status::AK_STATUS_BUDGET_BUSY)
        }
    }

    /// Charges `len` more to a lend already counted, unless that would pass the first threshold:
    /// the slack of the spare arena it took.
    pub(crate) fn hold_more(&self, len: usize) -> bool {
        self.add_bytes(len)
    }

    // Sequentially consistent, as `keep_spare` and the trim after it are: a charge adds to its
    // count and then reads the spares', a spare is kept and then the trim reads the charges, so
    // the second of the two sees both and gives the spares up.
    fn add_bytes(&self, len: usize) -> bool {
        let mut seen = self.bytes.load(Ordering::SeqCst);
        loop {
            let Some(wanted) = seen
                .checked_add(len as u64)
                .filter(|wanted| *wanted <= self.limit())
            else {
                return false;
            };
            match self
                .bytes
                .compare_exchange_weak(seen, wanted, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => {
                    self.trim_spares();
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
        if len > 0 {
            self.room_made();
        }
        self.release();
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
    pub(crate) fn hold_received(self: &Arc<Self>, len: usize) -> Option<Received> {
        self.hold();

        let mut seen = self.bytes.load(Ordering::SeqCst);
        loop {
            let Some(wanted) = seen
                .checked_add(len as u64)
                .filter(|wanted| *wanted <= self.hard_limit())
            else {
                self.release();
                return None;
            };
            match self
                .bytes
                .compare_exchange_weak(seen, wanted, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => {
                    self.trim_spares();
                    return Some(Received {
                        ledger: Arc::clone(self),
                        len,
                    });
                }
                Err(current) => seen = current,
            }
        }
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
