mod server;

// Every helper is used by one cardinality or another, so which ones a given test binary leaves
// unused is not a fact about the fixture.
#[allow(dead_code)]
pub mod host;

#[allow(dead_code)]
#[path = "../../../armonik-transport/tests/common/codec.rs"]
mod codec;
#[allow(dead_code)]
#[path = "../../../armonik-transport/tests/common/echo.rs"]
mod echo;

// Which method names a given test binary needs is a fact about that binary, not about the
// fixture that serves all of them.
#[allow(unused_imports)]
pub use echo::{flaky_seen, CHAT, COLLECT, ECHO, FAIL, FAN, FLAKY, FRAMES, SLOW};
pub use server::TestServer;

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use armonik_transport_ffi::*;

#[derive(Clone, Debug)]
pub struct Event {
    pub kind: ak_event_kind,
    pub payload: Vec<u8>,
    pub status_code: i32,
    pub host_debt: ak_host_debt,
    pub had_owner: bool,
    /// The name of the thread this callback ran on, which is what says where it came from.
    pub on_thread: Option<String>,
    /// The runtime's status, read from inside the callback of a runtime event.
    pub runtime_state_inside: Option<ak_runtime_state>,
    /// Which callback carried it, counted from one: events with the same number came together.
    pub callback: u64,
    owner: usize,
}

/// What a test has the host do from inside a callback, on each event once it is recorded.
pub type Reaction = Arc<dyn Fn(ak_event_kind) + Send + Sync>;

#[derive(Default)]
pub struct Recorder {
    seen: Mutex<Vec<Event>>,
    arrived: Condvar,
    holding: AtomicBool,
    runtime: AtomicU64,
    callbacks: AtomicU64,
    reaction: Mutex<Option<Reaction>>,
}

pub unsafe extern "C" fn on_event(
    runtime_ctx: *mut c_void,
    call_ctx: *mut c_void,
    events: *const ak_event,
    count: usize,
) {
    let recorder = unsafe { &*(runtime_ctx as *const Recorder) };
    assert!(count > 0, "a callback carries at least one event");
    let callback = recorder.callbacks.fetch_add(1, Ordering::AcqRel) + 1;
    for event in unsafe { std::slice::from_raw_parts(events, count) } {
        record_one(recorder, call_ctx, event, callback);
        recorder.react_to(event.kind);
    }
}

fn record_one(recorder: &Recorder, call_ctx: *mut c_void, event: &ak_event, callback: u64) {
    let owner = event.payload.owner;
    let payload = if event.payload.ptr.is_null() || event.payload.len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(event.payload.ptr, event.payload.len) }.to_vec()
    };

    let runtime_state_inside = call_ctx
        .is_null()
        .then(|| ak_runtime_status(recorder.runtime.load(Ordering::Acquire)));

    let holding = recorder.holding.load(Ordering::Acquire);
    recorder.record(Event {
        kind: event.kind,
        payload,
        status_code: event.status_code,
        host_debt: event.host_debt,
        had_owner: !owner.is_null(),
        on_thread: std::thread::current().name().map(str::to_owned),
        runtime_state_inside,
        callback,
        owner: if holding { owner as usize } else { 0 },
    });

    if !holding {
        unsafe { ak_event_consumed(event.payload) };
    }
}

impl Recorder {
    fn seen(&self) -> MutexGuard<'_, Vec<Event>> {
        self.seen.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn record(&self, event: Event) {
        self.seen().push(event);
        self.arrived.notify_all();
    }

    /// Has the host run `reaction` on every event from now on; `None` stops it.
    pub fn react(&self, reaction: Option<Reaction>) {
        *self.reaction.lock().unwrap_or_else(PoisonError::into_inner) = reaction;
    }

    // Cloned out of the lock before it runs, so a reaction that blocks holds no lock while it does.
    fn react_to(&self, kind: ak_event_kind) {
        let reaction = self
            .reaction
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(reaction) = reaction {
            reaction(kind);
        }
    }

    pub fn len(&self) -> usize {
        self.seen().len()
    }

    pub fn kinds(&self) -> Vec<ak_event_kind> {
        kinds(&self.seen())
    }

    pub fn shutdown_debt(&self) -> Option<ak_host_debt> {
        self.seen()
            .iter()
            .find(|event| event.kind == ak_event_kind::AK_EVENT_SHUTDOWN_COMPLETE)
            .map(|event| event.host_debt)
    }

    /// The last event of a kind, for a test that asks where one came from.
    pub fn last_of(&self, kind: ak_event_kind) -> Option<Event> {
        self.seen()
            .iter()
            .rev()
            .find(|event| event.kind == kind)
            .cloned()
    }

    /// The runtime whose status a runtime event's callback reads.
    pub fn watch_runtime(&self, runtime: ak_handle) {
        self.runtime.store(runtime, Ordering::Release);
    }

    pub fn hold_payloads(&self) {
        self.holding.store(true, Ordering::Release);
    }

    /// Back to consuming each payload as it arrives, for a fixture winding down after a failure:
    /// the shutdown waits for every payload to come back, and a recorder that keeps holding them
    /// keeps the runtime from ever reaching quiescence.
    pub fn stop_holding(&self) {
        self.holding.store(false, Ordering::Release);
    }

    fn wait_for(&self, what: &str, ready: impl Fn(&[Event]) -> bool) -> Seen {
        let (seen, waited) = self
            .arrived
            .wait_timeout_while(self.seen(), Duration::from_secs(10), |seen| !ready(seen))
            .unwrap_or_else(PoisonError::into_inner);

        if waited.timed_out() {
            let saw = kinds(&seen);
            drop(seen);
            panic!("waited for {what}, saw {saw:?}");
        }
        Seen(seen.clone())
    }

    fn await_kind(&self, what: &str, kind: ak_event_kind) -> Seen {
        self.wait_for(what, |seen| seen.iter().any(|event| event.kind == kind))
    }

    pub fn await_metadata(&self) -> Seen {
        self.await_kind("the metadata", ak_event_kind::AK_EVENT_INITIAL_METADATA)
    }

    pub fn await_terminal(&self) -> Seen {
        self.await_kind("a terminal", ak_event_kind::AK_EVENT_STATUS)
    }

    pub fn await_write_done(&self) -> Seen {
        self.await_kind("an acquittal", ak_event_kind::AK_EVENT_WRITE_DONE)
    }

    /// Waits until `wanted` messages have arrived, for the same reason as the acquittals.
    pub fn await_messages(&self, wanted: usize) -> Seen {
        self.wait_for(&format!("{wanted} message(s)"), |seen| {
            seen.iter()
                .filter(|event| event.kind == ak_event_kind::AK_EVENT_MESSAGE)
                .count()
                >= wanted
        })
    }

    /// Waits until `wanted` acquittals have arrived, which is what a caller sending several
    /// messages in a row needs: every event is kept, so waiting for the kind alone would be
    /// satisfied by the first one.
    pub fn await_write_dones(&self, wanted: usize) -> Seen {
        self.wait_for(&format!("{wanted} acquittal(s)"), |seen| {
            seen.iter()
                .filter(|event| event.kind == ak_event_kind::AK_EVENT_WRITE_DONE)
                .count()
                >= wanted
        })
    }

    pub fn await_budget_wake(&self) -> Seen {
        self.await_kind("a budget wake-up", ak_event_kind::AK_EVENT_BUDGET_WAKE)
    }

    /// Waits until `wanted` calls have their terminal.
    pub fn await_terminals(&self, wanted: usize) -> Seen {
        self.wait_for(&format!("{wanted} terminal(s)"), |seen| {
            seen.iter()
                .filter(|event| event.kind == ak_event_kind::AK_EVENT_STATUS)
                .count()
                >= wanted
        })
    }

    pub fn await_shutdown(&self) -> Seen {
        self.await_kind("a shutdown", ak_event_kind::AK_EVENT_SHUTDOWN_COMPLETE)
    }

    /// Gives back every payload held in one downcall, an unowned one among them.
    pub fn consume_all_together(&self) {
        let mut seen = self.seen();
        let mut payloads = vec![ak_bytes {
            ptr: std::ptr::null(),
            len: 0,
            owner: std::ptr::null_mut(),
        }];
        for event in seen.iter_mut().filter(|event| event.owner != 0) {
            payloads.push(ak_bytes {
                ptr: std::ptr::null(),
                len: 0,
                owner: event.owner as *mut c_void,
            });
            event.owner = 0;
        }
        unsafe { ak_events_consumed(payloads.as_ptr(), payloads.len()) };
    }

    pub fn consume_all(&self) {
        let mut seen = self.seen();
        for event in seen.iter_mut() {
            if event.owner != 0 {
                unsafe {
                    ak_event_consumed(ak_bytes {
                        ptr: std::ptr::null(),
                        len: 0,
                        owner: event.owner as *mut c_void,
                    })
                };
                event.owner = 0;
            }
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.consume_all();
    }
}

fn kinds(seen: &[Event]) -> Vec<ak_event_kind> {
    seen.iter().map(|event| event.kind).collect()
}

pub fn poll_until(ready: impl Fn() -> bool, diagnose: impl Fn() -> String) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if ready() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("{}", diagnose());
}

pub fn await_call_reclaimed(call: ak_handle) {
    poll_until(
        || unsafe { ak_call_cancel(call, std::ptr::null_mut()) } == ak_status::AK_STATUS_HANDLE_STALE,
        || "the call was not reclaimed".to_owned(),
    );
}

fn is_data(kind: ak_event_kind) -> bool {
    matches!(
        kind,
        ak_event_kind::AK_EVENT_INITIAL_METADATA
            | ak_event_kind::AK_EVENT_MESSAGE
            | ak_event_kind::AK_EVENT_STATUS
    )
}

#[derive(Debug)]
pub struct Seen(Vec<Event>);

impl Seen {
    pub fn kinds(&self) -> Vec<ak_event_kind> {
        kinds(&self.0)
    }

    /// The thread each event was delivered on, by name.
    pub fn threads(&self) -> Vec<Option<String>> {
        self.0.iter().map(|event| event.on_thread.clone()).collect()
    }

    /// How many callbacks carried the data events of the last call, from its head to its
    /// terminal.
    pub fn last_call_data_callbacks(&self) -> usize {
        let data: Vec<&Event> = self.0.iter().filter(|event| is_data(event.kind)).collect();
        let head = data
            .iter()
            .rposition(|event| event.kind == ak_event_kind::AK_EVENT_INITIAL_METADATA)
            .expect("a call's data starts with its head");
        let mut callbacks: Vec<u64> = data[head..].iter().map(|event| event.callback).collect();
        callbacks.dedup();
        callbacks.len()
    }

    pub fn data_kinds(&self) -> Vec<ak_event_kind> {
        self.0
            .iter()
            .map(|event| event.kind)
            .filter(|kind| is_data(*kind))
            .collect()
    }

    pub fn message_payloads(&self) -> Vec<Vec<u8>> {
        self.0
            .iter()
            .filter(|event| event.kind == ak_event_kind::AK_EVENT_MESSAGE)
            .map(|event| event.payload.clone())
            .collect()
    }

    fn terminal(&self) -> Option<&Event> {
        self.0
            .iter()
            .find(|event| event.kind == ak_event_kind::AK_EVENT_STATUS)
    }

    pub fn status_code(&self) -> Option<i32> {
        self.terminal().map(|event| event.status_code)
    }

    pub fn status_message(&self) -> String {
        let Some(terminal) = self.terminal() else {
            return String::new();
        };
        let message = take(&mut &terminal.payload[..]).unwrap_or_default();
        String::from_utf8_lossy(&message).into_owned()
    }

    pub fn initial_metadata(&self) -> HashMap<Vec<u8>, Vec<u8>> {
        self.0
            .iter()
            .find(|event| event.kind == ak_event_kind::AK_EVENT_INITIAL_METADATA)
            .map(|event| unblob(&event.payload))
            .unwrap_or_default()
    }

    pub fn first_data_event_was_owned(&self) -> bool {
        self.0
            .iter()
            .find(|event| is_data(event.kind))
            .is_some_and(|event| event.had_owner)
    }
}

pub fn empty_buffer() -> ak_buffer {
    ak_buffer {
        ptr: std::ptr::null_mut(),
        len: 0,
        owner: std::ptr::null_mut(),
    }
}

pub fn blob(pairs: &[(&[u8], &[u8])]) -> Vec<u8> {
    let mut out = (pairs.len() as u32).to_ne_bytes().to_vec();
    for (key, value) in pairs {
        out.extend_from_slice(&(key.len() as u32).to_ne_bytes());
        out.extend_from_slice(key);
        out.extend_from_slice(&(value.len() as u32).to_ne_bytes());
        out.extend_from_slice(value);
    }
    out
}

fn unblob(bytes: &[u8]) -> HashMap<Vec<u8>, Vec<u8>> {
    let mut out = HashMap::new();
    if bytes.len() < 4 {
        return out;
    }
    let mut cursor = &bytes[4..];
    let count = u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    for _ in 0..count {
        let Some(key) = take(&mut cursor) else { break };
        let Some(value) = take(&mut cursor) else {
            break;
        };
        out.insert(key, value);
    }
    out
}

fn take(cursor: &mut &[u8]) -> Option<Vec<u8>> {
    if cursor.len() < 4 {
        return None;
    }
    let len = u32::from_ne_bytes([cursor[0], cursor[1], cursor[2], cursor[3]]) as usize;
    if cursor.len() < 4 + len {
        return None;
    }
    let chunk = cursor[4..4 + len].to_vec();
    *cursor = &cursor[4 + len..];
    Some(chunk)
}
