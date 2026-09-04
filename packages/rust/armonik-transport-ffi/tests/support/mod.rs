//! What the tests need to play a host: a recorder for the callback, and a gRPC server to call.

mod server;

// The engine crate's test server, taken by path rather than copied: both suites have to face the
// same peer or a change to what it answers has to be made twice. It reaches everything through
// `armonik_transport::reexports`, so it compiles unchanged here, and neither suite uses all of it.
#[allow(dead_code)]
#[path = "../../../armonik-transport/tests/common/codec.rs"]
mod codec;
#[allow(dead_code)]
#[path = "../../../armonik-transport/tests/common/echo.rs"]
mod echo;

pub use echo::{ECHO, FAIL, SLOW};
pub use server::TestServer;

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use armonik_transport_ffi::*;

#[derive(Clone, Debug)]
/// One event, copied out of the callback so the test can look at it afterwards.
pub struct Event {
    pub kind: ak_event_kind,
    pub payload: Vec<u8>,
    pub status_code: i32,
    pub host_debt: ak_host_debt,
    /// Whether the event carried something to give back. An empty payload is not unowned.
    pub had_owner: bool,
    /// What `ak_event_consumed` still has to take back, as an address: a raw pointer is not
    /// `Send`, and this travels from a runtime thread to the test's. Zero once given back.
    owner: usize,
}

#[derive(Default)]
/// Every event a runtime has delivered, and the payloads not yet given back.
///
/// By default it consumes each payload as it records it, which is what a host does and what the
/// delivery credit requires: with one credit, a host that waits before consuming never sees the
/// next data event.
pub struct Recorder {
    seen: Mutex<Vec<Event>>,
    arrived: Condvar,
    holding: AtomicBool,
}

/// The callback the tests hand the ABI.
///
/// # Safety
///
/// `runtime_ctx` must be the `Recorder` given to `ak_runtime_create`, and `event` must point at a
/// live event for the duration of the call, which is what the ABI promises.
pub unsafe extern "C" fn on_event(
    runtime_ctx: *mut c_void,
    _call_ctx: *mut c_void,
    event: *const ak_event,
) {
    // SAFETY: forwarded from this function's own contract.
    let (recorder, event) = unsafe { (&*(runtime_ctx as *const Recorder), &*event) };

    let owner = event.payload.owner;
    let payload = if event.payload.ptr.is_null() || event.payload.len == 0 {
        Vec::new()
    } else {
        // SAFETY: the ABI says the view is readable for `len` bytes until it is consumed.
        unsafe { std::slice::from_raw_parts(event.payload.ptr, event.payload.len) }.to_vec()
    };

    let holding = recorder.holding.load(Ordering::Acquire);
    recorder.record(Event {
        kind: event.kind,
        payload,
        status_code: event.status_code,
        host_debt: event.host_debt,
        had_owner: !owner.is_null(),
        owner: if holding { owner as usize } else { 0 },
    });

    if !holding {
        // SAFETY: the payload is the one just delivered and this is its only consumption.
        unsafe { ak_event_consumed(event.payload) };
    }
}

impl Recorder {
    /// The events so far. The poison is taken rather than reported: a test that has already
    /// failed inside a callback must not turn every later read into a second panic.
    fn seen(&self) -> MutexGuard<'_, Vec<Event>> {
        self.seen.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn record(&self, event: Event) {
        self.seen().push(event);
        self.arrived.notify_all();
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

    /// Keeps the payloads instead of consuming them, so a test can watch what that owes.
    pub fn hold_payloads(&self) {
        self.holding.store(true, Ordering::Release);
    }

    fn wait_for(&self, what: &str, ready: impl Fn(&[Event]) -> bool) -> Seen {
        let (seen, waited) = self
            .arrived
            .wait_timeout_while(self.seen(), Duration::from_secs(10), |seen| !ready(seen))
            .unwrap_or_else(PoisonError::into_inner);

        if waited.timed_out() {
            // The guard is dropped before the panic: one held across the unwind poisons the
            // lock, and the next callback would then panic inside an `extern "C"`.
            let saw = kinds(&seen);
            drop(seen);
            panic!("waited for {what}, saw {saw:?}");
        }
        Seen(seen.clone())
    }

    /// Waits for one event of `kind`.
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

    pub fn await_shutdown(&self) -> Seen {
        self.await_kind("a shutdown", ak_event_kind::AK_EVENT_SHUTDOWN_COMPLETE)
    }

    /// Gives every payload back, which is what frees the memory and arms the next event.
    pub fn consume_all(&self) {
        let mut seen = self.seen();
        for event in seen.iter_mut() {
            if event.owner != 0 {
                // SAFETY: each owner is one this library delivered and this is its only
                // consumption; the address is zeroed so a second pass cannot repeat it.
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
    /// A test that fails part way still owes the payloads it took, and the runtime cannot
    /// reach quiescence until they come back.
    fn drop(&mut self) {
        self.consume_all();
    }
}

fn kinds(seen: &[Event]) -> Vec<ak_event_kind> {
    seen.iter().map(|event| event.kind).collect()
}

/// Polls `ready` until it holds, or gives up after ten seconds with `diagnose`'s account of why.
///
/// A poll and not a wait because neither of the two things asked here is announced: a reclaimed
/// handle and a runtime's state are both read through the ABI rather than delivered.
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

/// Waits for the runtime to take a call's handle back, which it does on its own once nothing of
/// the call is outstanding.
pub fn await_call_reclaimed(call: ak_handle) {
    poll_until(
        || ak_call_cancel(call) == ak_status::AK_STATUS_HANDLE_STALE,
        || "the call was not reclaimed".to_owned(),
    );
}

/// The events the ABI serializes among themselves. WRITE_DONE is a second domain and the
/// runtime's own two belong to no call.
fn is_data(kind: ak_event_kind) -> bool {
    matches!(
        kind,
        ak_event_kind::AK_EVENT_INITIAL_METADATA
            | ak_event_kind::AK_EVENT_MESSAGE
            | ak_event_kind::AK_EVENT_STATUS
    )
}

#[derive(Debug)]
/// A snapshot of what a runtime has delivered.
pub struct Seen(Vec<Event>);

impl Seen {
    pub fn kinds(&self) -> Vec<ak_event_kind> {
        kinds(&self.0)
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

    /// The reason the terminal carries: a length-prefixed message, then the trailing metadata.
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

    /// Whether the first data event carries something to give back. An empty payload is not an
    /// unowned one: the credit comes back with the acquittal and not with the bytes.
    pub fn first_data_event_was_owned(&self) -> bool {
        self.0
            .iter()
            .find(|event| is_data(event.kind))
            .is_some_and(|event| event.had_owner)
    }
}

/// A buffer as C would leave it before a lend fills it in. The ABI promises `*out` is untouched
/// on a refusal, so this is also what a host still holds after one.
pub fn empty_buffer() -> ak_buffer {
    ak_buffer {
        ptr: std::ptr::null_mut(),
        len: 0,
        owner: std::ptr::null_mut(),
    }
}

/// The blob encoding the ABI uses for every list of pairs.
///
/// Written out here rather than taken from the library: a test that builds its blob with the
/// encoder under test cannot catch that encoder changing.
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
