//! A safe wrapper over the `ak_*` entry points, and the reference for how to use them.
//!
//! A test reads as a sequence of request steps rather than a wall of `unsafe`. The rules the ABI
//! puts on a caller are all here in one place: one armed operation at a time, every event copied out
//! during the callback, the completion is terminal.
//!
//! Test bodies stay synchronous. The events arrive on the library's own runtime threads and are
//! pushed into an ordinary channel, which the test blocks on; driving the ABI from inside an `async`
//! block would mean occupying a thread the runtime also needs.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use armonik_transport_ffi::event::ak_event;
use armonik_transport_ffi::status::ak_status;
use armonik_transport_ffi::{ak_bytes, ak_bytes_in, ak_bytes_release};
use armonik_transport_ffi::{ak_client, ak_client_create, ak_client_release};
use armonik_transport_ffi::{ak_request, ak_request_close_send, ak_request_read};
use armonik_transport_ffi::{ak_request_release, ak_request_start, ak_request_write};

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

/// What the callback's `ctx` points at: the sending half of the test's event channel.
#[derive(Debug)]
struct Context {
    events: Sender<Event>,
}

/// The event callback itself.
///
/// Everything it does happens before it returns: the payload is borrowed, so a copy here is not an
/// optimisation to skip but the only way the bytes survive the call.
extern "C" fn on_event(ctx: *mut std::ffi::c_void, kind: i32, payload: ak_bytes_in, code: i32) {
    // SAFETY: `ctx` is the `Context` leaked by `Request::start`, which is never reclaimed.
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
    // A closed channel means the test has already finished and stopped listening.
    let _ = context.events.send(event);
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
    events: Receiver<Event>,
}

// SAFETY: as for `Client`. The tests need it because an event arrives on a runtime thread while the
// test thread is blocked on the channel.
unsafe impl Send for Request {}

impl Request {
    /// Open a request. `headers` must include `:method` and `:url`.
    pub(crate) fn start(client: &Client, headers: &[(&str, &str)]) -> Result<Self, (i32, String)> {
        let (tx, rx) = std::sync::mpsc::channel();
        // Leaked, and never reclaimed. Releasing silences the callback but does not wait for a
        // delivery already under way, so there is no moment at which this side could prove nothing
        // is inside it; a test binary is short-lived, and a leak here is cheaper than a race.
        let context = Box::into_raw(Box::new(Context { events: tx }));
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
            return Err((status, message));
        }
        Ok(Self { raw, events: rx })
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

impl Drop for Request {
    fn drop(&mut self) {
        // SAFETY: created by `ak_request_start` and released exactly once, here.
        unsafe { ak_request_release(self.raw) };
    }
}
