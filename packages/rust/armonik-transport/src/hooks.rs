//! Points where a test runs its own code inside the engine, to reach a path nothing else reaches,
//! and counts of what the engine writes and of the rounds its deliveries wait. Compiled with the
//! `test-hooks` feature only.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use crate::grpc::{GrpcStatusCode, Origin, Pushback};

/// What a hook runs on the thread that reaches it.
pub type Hook = Arc<dyn Fn() + Send + Sync>;

/// An attempt that went out and ended: where its end came from, its code, and what its server
/// said of a retry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attempt {
    pub origin: Origin,
    pub code: GrpcStatusCode,
    pub pushback: Pushback,
}

/// Where a channel's estimate of its server stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AdaptiveState {
    /// Attempts in the window that the server accepted.
    pub accepted: u64,
    /// Attempts in the window that failed transiently.
    pub transient: u64,
    /// Attempts in the window that said the server is over capacity.
    pub overloaded: u64,
    /// Whether retries are open: the retry reading is under the slack.
    pub retries_open: bool,
    /// The rate of first attempts the channel may start, a second, while it is capped.
    pub cap_per_second: Option<f64>,
}

/// A channel's estimate of its server, alone, for the benchmark of its hot path.
pub struct EstimateBench(crate::grpc::Adaptive);

impl EstimateBench {
    pub fn new(config: crate::grpc::AdaptiveConfig) -> Self {
        Self(crate::grpc::Adaptive::new(config))
    }

    /// Counts an attempt the server accepted.
    pub fn record_accept(&self) {
        self.0.record(crate::grpc::Class::Accept);
    }

    /// Counts an attempt that said the server is over capacity.
    pub fn record_overload(&self) {
        self.0.record(crate::grpc::Class::Overload);
    }

    /// The decision a retry meets at its failure.
    pub fn retries_open(&self) -> bool {
        self.0.retries_open()
    }

    /// The decision a first attempt meets, on the path where nothing is capped and nobody waits.
    pub async fn first_attempt(&self) {
        self.0.admit_first().await;
    }
}

/// What a hook runs on the thread that ends an attempt.
pub type AttemptHook = Arc<dyn Fn(&Attempt) + Send + Sync>;

static IN_DRIVER: Mutex<Option<Hook>> = Mutex::new(None);
static IN_DIAL: Mutex<Option<Hook>> = Mutex::new(None);
static ON_ATTEMPT: Mutex<Option<AttemptHook>> = Mutex::new(None);
static WRITES: AtomicUsize = AtomicUsize::new(0);
static DELIVERY_ROUNDS: AtomicUsize = AtomicUsize::new(0);
static COMPRESSIONS: AtomicUsize = AtomicUsize::new(0);

/// How many writes the HTTP/2 connections of this process have made to the stream under them,
/// TLS's when there is one: a write taken in parts counts each part.
pub fn writes() -> usize {
    WRITES.load(Ordering::SeqCst)
}

pub(crate) fn count_write() {
    WRITES.fetch_add(1, Ordering::SeqCst);
}

/// How many rounds of the runtime the deliveries of this process's responses have waited for
/// their next read.
pub fn delivery_rounds() -> usize {
    DELIVERY_ROUNDS.load(Ordering::SeqCst)
}

pub(crate) fn count_delivery_round() {
    DELIVERY_ROUNDS.fetch_add(1, Ordering::SeqCst);
}

/// How many messages the engine has begun to compress in this process, whether or not the
/// compression gained anything.
pub fn compressions() -> usize {
    COMPRESSIONS.load(Ordering::SeqCst)
}

pub(crate) fn count_compression() {
    COMPRESSIONS.fetch_add(1, Ordering::SeqCst);
}

/// Runs `hook` at the start of every call's driver. `None` removes it.
pub fn in_driver(hook: Option<Hook>) {
    set(&IN_DRIVER, hook);
}

/// Runs `hook` at the start of every dial, before the connection is opened. `None` removes it.
pub fn in_dial(hook: Option<Hook>) {
    set(&IN_DIAL, hook);
}

/// Runs `hook` at the end of every attempt that went out, a call's first, a retry or a resend. An
/// attempt skipped for want of a turn went nowhere, and one the call's deadline cuts short is not
/// told. `None` removes it.
pub fn on_attempt(hook: Option<AttemptHook>) {
    *ON_ATTEMPT.lock().unwrap_or_else(PoisonError::into_inner) = hook;
}

pub(crate) fn attempt_ended(origin: &Origin, code: GrpcStatusCode, pushback: Pushback) {
    let hook = ON_ATTEMPT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    if let Some(hook) = hook {
        hook(&Attempt {
            origin: origin.clone(),
            code,
            pushback,
        });
    }
}

pub(crate) fn run_in_driver() {
    run(&IN_DRIVER);
}

pub(crate) fn run_in_dial() {
    run(&IN_DIAL);
}

fn set(point: &Mutex<Option<Hook>>, hook: Option<Hook>) {
    *point.lock().unwrap_or_else(PoisonError::into_inner) = hook;
}

// Cloned out of the lock before it runs, so a hook that panics leaves the lock unpoisoned.
fn run(point: &Mutex<Option<Hook>>) {
    let hook = point.lock().unwrap_or_else(PoisonError::into_inner).clone();
    if let Some(hook) = hook {
        hook();
    }
}
