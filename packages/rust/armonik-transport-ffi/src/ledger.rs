use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::watch;

use crate::abi::{ak_memory_usage, ak_status};

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
            ceiling: self.ceiling,
        }
    }

    fn limit(&self) -> u64 {
        if self.ceiling == 0 {
            u64::MAX
        } else {
            self.ceiling
        }
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
