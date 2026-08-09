//! A safe wrapper over the `ak_*` entry points, and the reference for how to use them.
//!
//! A test reads as a sequence of request steps rather than a wall of `unsafe`. The rules the ABI
//! puts on a caller are all here in one place: one armed operation at a time, every event copied out
//! during the callback, the completion is terminal.
//!
//! Test bodies stay synchronous. The events arrive on the library's own runtime threads and are
//! pushed into an ordinary channel, which the test blocks on; driving the ABI from inside an `async`
//! block would mean occupying a thread the runtime also needs.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

use armonik_transport_ffi::ak_request_write;
use armonik_transport_ffi::event::ak_event;
use armonik_transport_ffi::status::ak_status;
use armonik_transport_ffi::{ak_bytes, ak_bytes_in, ak_bytes_release};
use armonik_transport_ffi::{ak_client, ak_client_create, ak_client_release};
use armonik_transport_ffi::{ak_request, ak_request_cancel, ak_request_close_send};
use armonik_transport_ffi::{ak_request_read, ak_request_release, ak_request_start};

/// How long a test waits for an event before deciding one is not coming.
pub(crate) const PATIENCE: Duration = Duration::from_secs(10);

/// `AK_OK`, spelled once so a test asserting success does not have to cast.
pub(crate) const OK: i32 = ak_status::AK_OK as i32;

/// One delivered event, with everything copied out of the borrowed payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Event {
    Headers(Vec<(Vec<u8>, Vec<u8>)>),
    WriteDone,
    Read(Vec<u8>),
    /// `code` is `AK_OK` with the trailers, or a failure with the message.
    Completed {
        code: i32,
        trailers: Vec<(Vec<u8>, Vec<u8>)>,
        message: String,
    },
}

impl Event {
    /// The completed status and message, or a panic naming what arrived instead.
    pub(crate) fn expect_completed(&self) -> (i32, &str) {
        match self {
            Event::Completed { code, message, .. } => (*code, message.as_str()),
            other => panic!("expected the completion, got {other:?}"),
        }
    }

    /// The `:status` of a headers event, or a panic naming what arrived instead.
    pub(crate) fn expect_http_status(&self) -> String {
        let Event::Headers(pairs) = self else {
            panic!("expected the response headers, got {self:?}");
        };
        pairs
            .iter()
            .find(|(key, _)| key == b":status")
            .map(|(_, value)| String::from_utf8_lossy(value).into_owned())
            .expect("`:status` is always in the headers blob")
    }
}

/// What the callback's `ctx` points at: the sending half of the test's event channel, and the flag
/// that says this context has been given back.
#[derive(Debug)]
struct Context {
    events: Sender<Event>,
    reclaimed: Arc<AtomicBool>,
}

impl Drop for Context {
    fn drop(&mut self) {
        self.reclaimed.store(true, Ordering::SeqCst);
    }
}

/// The event callback itself.
///
/// Everything it does happens before it returns: the payload is borrowed, so a copy here is not an
/// optimisation to skip but the only way the bytes survive the call. And the completion is where the
/// context comes back, which is the whole of the ownership rule a consumer has to follow.
extern "C" fn on_event(ctx: *mut std::ffi::c_void, kind: i32, payload: ak_bytes_in, code: i32) {
    // SAFETY: `ctx` is the `Context` leaked by `Request::start`, and the ABI keeps it valid until
    // the completion, which is the last callback ever made for this request.
    let context = unsafe { &*ctx.cast::<Context>() };
    let bytes = if payload.ptr.is_null() || payload.len == 0 {
        &[][..]
    } else {
        // SAFETY: the ABI documents the payload as readable for the duration of this call.
        unsafe { std::slice::from_raw_parts(payload.ptr, payload.len) }
    };

    let event = if kind == ak_event::AK_EVENT_RESPONSE_HEADERS as i32 {
        Event::Headers(decode_blob(bytes))
    } else if kind == ak_event::AK_EVENT_WRITE_DONE as i32 {
        Event::WriteDone
    } else if kind == ak_event::AK_EVENT_READ_DONE as i32 {
        Event::Read(bytes.to_vec())
    } else if kind == ak_event::AK_EVENT_COMPLETED as i32 {
        Event::Completed {
            code,
            trailers: if code == OK {
                decode_blob(bytes)
            } else {
                Vec::new()
            },
            message: if code == OK {
                String::new()
            } else {
                String::from_utf8_lossy(bytes).into_owned()
            },
        }
    } else {
        panic!("unknown event kind {kind}");
    };
    let terminal = matches!(event, Event::Completed { .. });
    // A closed channel means the test has already finished and stopped listening.
    let _ = context.events.send(event);

    if terminal {
        // Reclaimed here, and only here. The completion arrives on every path - a clean end, a
        // failure, a cancellation, a release before any of them - and nothing is delivered for this
        // request afterwards, so this is both the last use of the context and the only place that
        // has to give it back.
        // SAFETY: leaked exactly once in `start`, reclaimed exactly once here.
        drop(unsafe { Box::from_raw(ctx.cast::<Context>()) });
    }
}

/// Decode the ABI's key/value blob: `u32` count, then length-prefixed pairs, native byte order.
pub(crate) fn decode_blob(bytes: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
    if bytes.is_empty() {
        return Vec::new();
    }
    let mut cursor = bytes;
    let mut take = |count: usize| -> Vec<u8> {
        let (head, tail) = cursor.split_at(count);
        cursor = tail;
        head.to_vec()
    };
    let count = u32::from_ne_bytes(take(4).try_into().expect("four bytes")) as usize;
    (0..count)
        .map(|_| {
            let key_len = u32::from_ne_bytes(take(4).try_into().expect("four bytes")) as usize;
            let key = take(key_len);
            let value_len = u32::from_ne_bytes(take(4).try_into().expect("four bytes")) as usize;
            let value = take(value_len);
            (key, value)
        })
        .collect()
}

/// Encode a key/value blob the same way.
pub(crate) fn encode_blob(pairs: &[(&str, &str)]) -> Vec<u8> {
    let mut out = (pairs.len() as u32).to_ne_bytes().to_vec();
    for (key, value) in pairs {
        out.extend_from_slice(&(key.len() as u32).to_ne_bytes());
        out.extend_from_slice(key.as_bytes());
        out.extend_from_slice(&(value.len() as u32).to_ne_bytes());
        out.extend_from_slice(value.as_bytes());
    }
    out
}

/// A client handle, released on drop.
pub(crate) struct Client(*mut ak_client);

// SAFETY: the header states that a handle may be used from any thread, and from several at once; the
// pointer is an opaque token this side never dereferences.
unsafe impl Send for Client {}

impl Client {
    /// Build a client for `endpoint`, panicking with the ABI's own message if it is refused.
    pub(crate) fn new(endpoint: &str) -> Self {
        Self::try_new(&format!(r#"{{"Endpoint": "{endpoint}"}}"#)).expect("create the client")
    }

    /// Build a client from a whole configuration document.
    pub(crate) fn try_new(config: &str) -> Result<Self, (i32, String)> {
        let mut raw: *mut ak_client = std::ptr::null_mut();
        let mut err = ak_bytes::EMPTY;
        // SAFETY: both out-parameters are live locals and the document outlives the call.
        let status = unsafe {
            ak_client_create(
                config.as_ptr(),
                config.len(),
                std::ptr::addr_of_mut!(raw),
                std::ptr::addr_of_mut!(err),
            )
        };
        let message = take_message(err);
        if status == OK {
            Ok(Self(raw))
        } else {
            Err((status, message))
        }
    }

    pub(crate) fn as_ptr(&self) -> *const ak_client {
        self.0
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // SAFETY: created by `ak_client_create` and released exactly once, here.
        unsafe { ak_client_release(self.0) };
    }
}

/// Read an `ak_bytes` out and release it.
fn take_message(bytes: ak_bytes) -> String {
    if bytes.ptr.is_null() {
        return String::new();
    }
    // SAFETY: produced by the ABI, read once and released immediately.
    let seen = unsafe { std::slice::from_raw_parts(bytes.ptr, bytes.len) }.to_vec();
    // SAFETY: as above.
    unsafe { ak_bytes_release(bytes) };
    String::from_utf8_lossy(&seen).into_owned()
}

/// A request handle and the events it has produced.
#[derive(Debug)]
pub(crate) struct Request {
    raw: *mut ak_request,
    /// Whether the handle has been given back already, so that dropping the wrapper cannot do it a
    /// second time.
    released: std::cell::Cell<bool>,
    /// Raised when this request's context is given back.
    ///
    /// One flag per request rather than a count for the process: an assertion about a batch has to
    /// be about that batch, or a test added later - spawning its own requests on the same runtime -
    /// quietly turns it into an assertion about the whole binary, with a green tick over it.
    reclaimed: Arc<AtomicBool>,
    events: Receiver<Event>,
}

// SAFETY: as for `Client`. The tests need it because an event arrives on a runtime thread while the
// test thread is blocked on the channel.
unsafe impl Send for Request {}

impl Request {
    /// Open a request. `headers` must include `:method` and `:url`.
    pub(crate) fn start(client: &Client, headers: &[(&str, &str)]) -> Result<Self, (i32, String)> {
        let (tx, rx) = std::sync::mpsc::channel();
        // Handed to the ABI, which keeps it valid until the completion and gives it back there.
        let reclaimed = Arc::new(AtomicBool::new(false));
        let context = Box::into_raw(Box::new(Context {
            events: tx,
            reclaimed: Arc::clone(&reclaimed),
        }));
        let blob = encode_blob(headers);

        let mut raw: *mut ak_request = std::ptr::null_mut();
        let mut err = ak_bytes::EMPTY;
        // SAFETY: the blob outlives the call, the out-parameters are live locals, and `context` is
        // leaked so it outlives everything.
        let status = unsafe {
            ak_request_start(
                client.as_ptr(),
                blob.as_ptr(),
                blob.len(),
                Some(on_event),
                context.cast(),
                std::ptr::addr_of_mut!(raw),
                std::ptr::addr_of_mut!(err),
            )
        };
        let message = take_message(err);
        if status != OK {
            // A start that failed delivers no event ever, so the completion will not be giving this
            // back: it comes back here instead.
            // SAFETY: leaked just above and never handed anywhere else.
            drop(unsafe { Box::from_raw(context) });
            return Err((status, message));
        }
        Ok(Self {
            raw,
            released: std::cell::Cell::new(false),
            reclaimed,
            events: rx,
        })
    }

    /// Arm one write.
    pub(crate) fn write(&self, data: &[u8]) -> i32 {
        // SAFETY: `raw` is live and `data` outlives the call.
        unsafe { ak_request_write(self.raw, data.as_ptr(), data.len()) }
    }

    /// End the request body.
    pub(crate) fn close_send(&self) -> i32 {
        // SAFETY: `raw` is live.
        unsafe { ak_request_close_send(self.raw) }
    }

    /// Arm one read.
    pub(crate) fn read(&self) -> i32 {
        // SAFETY: `raw` is live.
        unsafe { ak_request_read(self.raw) }
    }

    /// Cancel the request.
    pub(crate) fn cancel(&self) -> i32 {
        // SAFETY: `raw` is live.
        unsafe { ak_request_cancel(self.raw) }
    }

    /// The bare handle, for a test that calls the ABI from another thread.
    pub(crate) fn raw(&self) -> RawRequest {
        RawRequest(self.raw)
    }

    /// Give up the handle without waiting for the completion, the way a consumer abandoning a
    /// request does.
    ///
    /// The wrapper stays alive because the events do: releasing cancels the request, and its
    /// completion still arrives. Idempotent, so dropping the wrapper afterwards gives nothing back
    /// twice.
    pub(crate) fn release(&self) {
        if !self.released.replace(true) {
            // SAFETY: created by `ak_request_start` and released exactly once.
            unsafe { ak_request_release(self.raw) };
        }
    }

    /// Whether the context this request was started with has been given back.
    ///
    /// The callback gives it up at the completion, which is the driving task's last act, so a
    /// context that has come back is a task that ended rather than one parked for ever.
    pub(crate) fn context_reclaimed(&self) -> bool {
        self.reclaimed.load(Ordering::SeqCst)
    }

    /// Wait for the next event.
    pub(crate) fn next_event(&self) -> Event {
        match self.events.recv_timeout(PATIENCE) {
            Ok(event) => event,
            Err(RecvTimeoutError::Timeout) => panic!("no event arrived within {PATIENCE:?}"),
            Err(RecvTimeoutError::Disconnected) => panic!("the event channel closed"),
        }
    }

    /// Whether any event arrives within `patience`. Used to prove that none does.
    pub(crate) fn try_next_event(&self, patience: Duration) -> Option<Event> {
        self.events.recv_timeout(patience).ok()
    }

    /// Arm a read and collect chunks until the request completes, returning the body and the
    /// terminal event.
    pub(crate) fn drain(&self) -> (Vec<u8>, Event) {
        let mut body = Vec::new();
        loop {
            assert_eq!(self.read(), OK);
            match self.next_event() {
                Event::Read(chunk) => body.extend_from_slice(&chunk),
                terminal @ Event::Completed { .. } => return (body, terminal),
                other => panic!("expected a read or the completion, got {other:?}"),
            }
        }
    }
}

/// The bare handle, for a test that calls the ABI from two threads at once.
///
/// [`Request`] itself is deliberately not `Sync`: it owns a `std::sync::mpsc::Receiver`, which is
/// not safe to read from two threads, and saying otherwise to buy one test a shortcut would be a lie
/// about the wrapper rather than a statement about the ABI. This is the part the header really does
/// guarantee - the handle - and nothing else.
#[derive(Clone, Copy)]
pub(crate) struct RawRequest(*mut ak_request);

// SAFETY: the header states that a handle may be used from any thread and from several at once,
// `_release` included.
unsafe impl Send for RawRequest {}
// SAFETY: as above.
unsafe impl Sync for RawRequest {}

impl RawRequest {
    /// Arm one read.
    pub(crate) fn read(self) -> i32 {
        // SAFETY: the handle is either live, or released and therefore refused by the ABI.
        unsafe { ak_request_read(self.0) }
    }

    /// Arm one write.
    pub(crate) fn write(self, data: &[u8]) -> i32 {
        // SAFETY: as above; `data` outlives the call.
        unsafe { ak_request_write(self.0, data.as_ptr(), data.len()) }
    }

    /// Cancel the request.
    pub(crate) fn cancel(self) -> i32 {
        // SAFETY: as above.
        unsafe { ak_request_cancel(self.0) }
    }
}

impl Drop for Request {
    fn drop(&mut self) {
        self.release();
    }
}
