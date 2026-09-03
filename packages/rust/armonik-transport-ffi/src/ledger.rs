//! What the host holds of a runtime, and the ceiling it is held against.
//!
//! One concept: a count that decides quiescence and a byte total that decides admission. It
//! knows nothing of calls, channels or the runtime that owns it - whoever charges it says how
//! much, and whoever waits on it asks whether it is empty.

use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::watch;

use crate::abi::{ak_memory_usage, ak_status};

/// What the host holds of a runtime: payloads not consumed, buffers not given back.
///
/// Quiescence is this reaching zero after the runtime has stopped, which is why the host gets
/// there by acting rather than by waiting.
pub(crate) struct Ledger {
    /// Payloads and buffers together: what decides quiescence. A zero-length payload is
    /// still something the host holds, which is why this counts and does not weigh.
    outstanding: AtomicU64,
    /// Lent buffers only: what the ceiling bounds.
    bytes: AtomicU64,
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

    pub(crate) fn hold(&self) {
        self.outstanding.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn release(&self) {
        self.outstanding.fetch_sub(1, Ordering::AcqRel);
        self.changed.send_modify(|version| *version += 1);
    }

    /// Whether a request of `len` could ever fit. A refusal here is permanent: no return by
    /// anyone will make room, so retrying is pointless.
    pub(crate) fn could_ever_fit(&self, len: usize) -> Result<(), ak_status> {
        match self.ceiling {
            0 => Ok(()),
            ceiling if len as u64 <= ceiling => Ok(()),
            _ => Err(ak_status::AK_STATUS_MESSAGE_TOO_LARGE),
        }
    }

    /// Takes `len` bytes against the ceiling, or reports that they are not there yet.
    pub(crate) fn reserve(&self, len: usize) -> Result<(), ak_status> {
        if self.ceiling == 0 {
            self.bytes.fetch_add(len as u64, Ordering::AcqRel);
            self.hold();
            return Ok(());
        }

        let mut seen = self.bytes.load(Ordering::Acquire);
        loop {
            let wanted = seen + len as u64;
            if wanted > self.ceiling {
                return Err(ak_status::AK_STATUS_BUDGET_BUSY);
            }
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
        let mut changed = self.changed.subscribe();
        while !self.empty() {
            if changed.changed().await.is_err() {
                return;
            }
        }
    }
}
