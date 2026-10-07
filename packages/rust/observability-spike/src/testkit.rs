//! Callbacks for tests and benchmarks.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use crate::record::{LogRecord, OwnedRecord};

/// Keeps a copy of every record it is handed.
#[derive(Default)]
pub struct Collector {
    pub records: Mutex<Vec<OwnedRecord>>,
}

impl Collector {
    pub fn ctx(&self) -> *mut c_void {
        self as *const Collector as *mut c_void
    }

    pub fn messages(&self) -> Vec<String> {
        self.records
            .lock()
            .unwrap()
            .iter()
            .map(|record| record.message.clone())
            .collect()
    }

    pub fn take(&self) -> Vec<OwnedRecord> {
        std::mem::take(&mut *self.records.lock().unwrap())
    }
}

pub unsafe extern "C" fn collect(ctx: *mut c_void, record: *const LogRecord) {
    let collector = unsafe { &*(ctx as *const Collector) };
    let owned = unsafe { OwnedRecord::copy_of(&*record) };
    collector.records.lock().unwrap().push(owned);
}

/// Counts and does nothing else: the cheapest callback a host could register.
#[derive(Default)]
pub struct Counter {
    pub count: AtomicU64,
}

impl Counter {
    pub fn ctx(&self) -> *mut c_void {
        self as *const Counter as *mut c_void
    }
}

pub unsafe extern "C" fn count(ctx: *mut c_void, _: *const LogRecord) {
    let counter = unsafe { &*(ctx as *const Counter) };
    counter.count.fetch_add(1, Ordering::Relaxed);
}
