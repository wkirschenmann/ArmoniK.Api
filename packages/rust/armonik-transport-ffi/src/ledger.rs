use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::watch;

use crate::abi::{ak_memory_usage, ak_status};

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

pub(crate) struct Ledger {
    outstanding: AtomicU64,
    bytes: AtomicU64,
    ceiling: u64,
    changed: watch::Sender<u64>,
}

impl Ledger {
    pub(crate) fn new(ceiling: u64) -> Self {
        Self {
            outstanding: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            ceiling,
            changed: watch::channel(0).0,
        }
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

    pub(crate) fn hold(&self) {
        self.outstanding.fetch_add(1, Ordering::AcqRel);
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

        let mut seen = self.bytes.load(Ordering::Acquire);
        loop {
            let Some(wanted) = seen
                .checked_add(len as u64)
                .filter(|wanted| *wanted <= self.limit())
            else {
                self.release();
                return Err(ak_status::AK_STATUS_BUDGET_BUSY);
            };
            match self.bytes.compare_exchange_weak(
                seen,
                wanted,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(()),
                Err(current) => seen = current,
            }
        }
    }

    pub(crate) fn release_bytes(&self, len: usize) {
        self.bytes.fetch_sub(len as u64, Ordering::AcqRel);
        self.release();
    }

    pub(crate) fn empty(&self) -> bool {
        self.outstanding.load(Ordering::Acquire) == 0
    }

    pub(crate) async fn drained(&self) {
        let _ = self.changed.subscribe().wait_for(|_| self.empty()).await;
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
        let ledger = Ledger::new(64);

        assert!(ledger.empty());
        assert_eq!(
            ledger.hold_bytes(65),
            Err(ak_status::AK_STATUS_BUDGET_BUSY)
        );
        assert!(ledger.empty(), "the refusal gave its count back");
        assert_eq!(ledger.usage().bytes_used, 0);
    }

    /// And what it counts is raised before the bytes it charges, which is the order `empty` reads.
    #[test]
    fn a_charge_is_counted_while_it_is_held() {
        let ledger = Ledger::new(64);

        assert_eq!(ledger.hold_bytes(40), Ok(()));
        assert!(!ledger.empty());
        assert_eq!(ledger.usage().bytes_used, 40);

        ledger.release_bytes(40);
        assert!(ledger.empty());
        assert_eq!(ledger.usage().bytes_used, 0);
    }
}
