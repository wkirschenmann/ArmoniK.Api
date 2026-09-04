//! What the host holds of a runtime, and the ceiling it is held against.
//!
//! Two axes, not two spellings of one. The count is every allocation the host has been given and
//! not yet given back, and reaching zero after the runtime has stopped is what quiescence means.
//! The byte total is lent buffers only - the ceiling bounds what the host may be filling, and a
//! delivered payload weighs nothing against it - so a payload moves the count alone.
//!
//! Hence two pairs, and the rule is that each is used whole: [`Ledger::hold_bytes`] with
//! [`Ledger::release_bytes`] for a buffer, [`Ledger::hold`] with [`Ledger::release`] for a
//! payload. Crossing them either leaks capacity against the ceiling for the life of the runtime
//! or underflows the total.
//!
//! It knows nothing of calls, channels or the runtime that owns it: whoever charges it says how
//! much, and whoever waits on it asks whether it is empty.

use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::watch;

use crate::abi::{ak_memory_usage, ak_status};

/// What the host holds of a runtime: payloads not consumed, buffers not given back.
pub(crate) struct Ledger {
    /// Payloads and buffers together: what decides quiescence. A zero-length payload is
    /// still something the host holds, which is why this counts and does not weigh.
    outstanding: AtomicU64,
    /// Lent buffers only: what the ceiling bounds.
    bytes: AtomicU64,
    /// Zero configures no ceiling. Kept as the host set it, because that is what
    /// [`Ledger::usage`] promises to report.
    ceiling: u64,
    /// Bumped on every release. A version and not a `Notify`: a waiter that checks its
    /// condition before creating the future misses a `notify_waiters` landing in between,
    /// and here that costs the runtime its quiescence for good.
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
            ceiling: self.ceiling,
        }
    }

    /// The ceiling as arithmetic: no ceiling is one nothing can reach, so the admission test is
    /// the same comparison either way.
    fn limit(&self) -> u64 {
        if self.ceiling == 0 {
            u64::MAX
        } else {
            self.ceiling
        }
    }

    /// Counts one thing the host holds.
    pub(crate) fn hold(&self) {
        self.outstanding.fetch_add(1, Ordering::AcqRel);
    }

    /// Gives one held thing back.
    ///
    /// Announced only on the return that empties the ledger: that is the one transition
    /// [`Ledger::drained`] waits for, and every other release would take the write lock on a
    /// cell shared by every call of the runtime to tell nobody anything.
    pub(crate) fn release(&self) {
        if self.outstanding.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.changed.send_modify(|version| *version += 1);
        }
    }

    /// Whether a request of `len` could ever fit. A refusal here is permanent: no return by
    /// anyone will make room, so retrying is pointless.
    pub(crate) fn could_ever_fit(&self, len: usize) -> Result<(), ak_status> {
        if len as u64 <= self.limit() {
            Ok(())
        } else {
            Err(ak_status::AK_STATUS_MESSAGE_TOO_LARGE)
        }
    }

    /// Takes `len` bytes against the ceiling and counts the buffer, or reports that the bytes
    /// are not there yet.
    pub(crate) fn hold_bytes(&self, len: usize) -> Result<(), ak_status> {
        let mut seen = self.bytes.load(Ordering::Acquire);
        loop {
            let Some(wanted) = seen
                .checked_add(len as u64)
                .filter(|wanted| *wanted <= self.limit())
            else {
                return Err(ak_status::AK_STATUS_BUDGET_BUSY);
            };
            match self.bytes.compare_exchange_weak(
                seen,
                wanted,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    self.hold();
                    return Ok(());
                }
                Err(current) => seen = current,
            }
        }
    }

    /// Gives `len` bytes back to the ceiling, and the buffer back to the count.
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
