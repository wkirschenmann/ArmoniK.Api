//! A safe wrapper over the `ak_*` entry points, and the reference for how to use them.
//!
//! A test reads as a sequence of request steps rather than a wall of `unsafe`. The rules the ABI
//! puts on a caller are all here in one place: one armed operation at a time, every event copied
//! out during the callback, `COMPLETED` releases the context.
//!
//! Test bodies stay synchronous. The events arrive on this crate's runtime threads and are pushed
//! into an ordinary channel, which the test blocks on; driving the ABI from inside an `async` block
//! would mean polling it from a thread the runtime also needs.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use armonik_transport_ffi::{ak_bytes, ak_bytes_free, ak_bytes_in, ak_client, ak_request};
use armonik_transport_ffi::{ak_client_create, ak_client_release};
use armonik_transport_ffi::{ak_request_cancel, ak_request_close_send, ak_request_release};
use armonik_transport_ffi::{ak_request_read, ak_request_start, ak_request_write};

/// How long a test waits for an event before deciding one is not coming.
pub(crate) const PATIENCE: Duration = Duration::from_secs(10);

/// One delivered event, with everything copied out of the borrowed payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Event {
    Headers(Vec<(Vec<u8>, Vec<u8>)>),
    WriteDone,
    Read(Vec<u8>),
    /// `code` is `AK_OK` with the trailers, or a negative status with the message.
    Completed {
        code: i32,
        trailers: Vec<(Vec<u8>, Vec<u8>)>,
        message: String,
    },
}

impl Event {
    /// The completed status, or a panic naming what arrived instead.
    pub(crate) fn expect_completed(&self) -> (i32, &str) {
        match self {
            Event::Completed { code, message, .. } => (*code, message.as_str()),
            other => panic!("expected COMPLETED, got {other:?}"),
        }
    }
}

/// What the callback's `ctx` points at: the sending half of the test's event channel.
///
/// Leaked on `start` and reclaimed by the `COMPLETED` callback. That is the ABI's rule, and it is
/// the reference count made explicit: the context belongs to the driving task from the moment the
/// request starts, and the terminal event is the task handing it back. Releasing the handle does
/// not end that - the events keep coming until `COMPLETED`.
#[derive(Debug)]
struct Context {
    events: Sender<Event>,
}

/// The event callback itself.
///
/// Everything it does happens before it returns: the payload is borrowed, so a copy here is not an
/// optimisation to skip but the only way the bytes survive the call.
extern "C" fn on_event(ctx: *mut std::ffi::c_void, kind: i32, payload: ak_bytes_in, code: i32) {
    // SAFETY: `ctx` is the `Context` leaked by `Request::start`. It stays alive until the
    // `COMPLETED` branch below gives it back, which the ABI guarantees is the last callback.
    let context = unsafe { &*ctx.cast::<Context>() };
    let bytes = if payload.ptr.is_null() || payload.len == 0 {
        &[][..]
    } else {
        // SAFETY: the ABI documents the payload as readable for the duration of this call.
        unsafe { std::slice::from_raw_parts(payload.ptr, payload.len) }
    };

    let event = match kind {
        armonik_transport_ffi::event::RESPONSE_HEADERS => Event::Headers(decode_blob(bytes)),
        armonik_transport_ffi::event::WRITE_DONE => Event::WriteDone,
        armonik_transport_ffi::event::READ_DONE => Event::Read(bytes.to_vec()),
        armonik_transport_ffi::event::COMPLETED => Event::Completed {
            code,
            trailers: if code == armonik_transport_ffi::status::OK {
                decode_blob(bytes)
            } else {
                Vec::new()
            },
            message: if code == armonik_transport_ffi::status::OK {
                String::new()
            } else {
                String::from_utf8_lossy(bytes).into_owned()
            },
        },
        other => panic!("unknown event kind {other}"),
    };
    let terminal = matches!(event, Event::Completed { .. });
    // A closed channel means the test has already finished and stopped listening.
    let _ = context.events.send(event);

    if terminal {
        // The last callback for this request, so the context comes back here. The borrow above ends
        // first; nothing reads it after this.
        // SAFETY: leaked by `Request::start`, reclaimed exactly once, and only ever on the one event
        // the ABI guarantees is both terminal and unique.
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

/// A client handle, freed on drop.
pub(crate) struct Client(*mut ak_client);

// SAFETY: the header states that a handle may be used from any thread, and from several at once;
// the pointer is an opaque token this side never dereferences.
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
        if status == armonik_transport_ffi::status::OK {
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

/// Read an `ak_bytes` out and free it.
fn take_message(bytes: ak_bytes) -> String {
    if bytes.ptr.is_null() {
        return String::new();
    }
    // SAFETY: produced by the ABI, read once and freed immediately.
    let seen = unsafe { std::slice::from_raw_parts(bytes.ptr, bytes.len) }.to_vec();
    // SAFETY: as above.
    unsafe { ak_bytes_free(bytes) };
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
        let context = Box::into_raw(Box::new(Context { events: tx }));
        let blob = encode_blob(headers);

        let mut raw: *mut ak_request = std::ptr::null_mut();
        let mut err = ak_bytes::EMPTY;
        // SAFETY: the blob outlives the call, the out-parameters are live locals, and `context`
        // stays valid until this wrapper is dropped.
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
        if status != armonik_transport_ffi::status::OK {
            // A start that failed produces no event ever, so the context comes back here instead.
            // SAFETY: leaked just above and never handed anywhere else.
            drop(unsafe { Box::from_raw(context) });
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

    /// Cancel the request.
    pub(crate) fn cancel(&self) -> i32 {
        // SAFETY: `raw` is live.
        unsafe { ak_request_cancel(self.raw) }
    }

    /// The bare handle, for a test that needs to call from another thread.
    pub(crate) fn raw(&self) -> RawRequest {
        RawRequest(self.raw)
    }

    /// Release the handle but keep listening, so a test can see what still arrives afterwards.
    pub(crate) fn release_and_keep_listening(self) -> Receiver<Event> {
        // SAFETY: created by `ak_request_start` and released exactly once, here; `Drop` is skipped
        // because the receiver is moved out.
        unsafe { ak_request_release(self.raw) };
        let this = std::mem::ManuallyDrop::new(self);
        // SAFETY: moved out of a value whose destructor will not run.
        unsafe { std::ptr::read(&this.events) }
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
            assert_eq!(self.read(), armonik_transport_ffi::status::OK);
            match self.next_event() {
                Event::Read(chunk) => body.extend_from_slice(&chunk),
                terminal @ Event::Completed { .. } => return (body, terminal),
                other => panic!("expected a read or a completion, got {other:?}"),
            }
        }
    }
}

/// The bare handle, for a test that calls the ABI from two threads at once.
///
/// [`Request`] itself is deliberately not `Sync`: it owns a `std::sync::mpsc::Receiver`, which is
/// not safe to read from two threads, and saying otherwise to buy one test a shortcut would be a
/// lie about the wrapper rather than a statement about the ABI. This is the part the header really
/// does guarantee - the handle - and nothing else.
#[derive(Clone, Copy)]
pub(crate) struct RawRequest(*mut ak_request);

// SAFETY: the header states that a handle may be used from any thread and from several at once,
// `_free` included.
unsafe impl Send for RawRequest {}
// SAFETY: as above.
unsafe impl Sync for RawRequest {}

impl RawRequest {
    /// Arm one read.
    pub(crate) fn read(self) -> i32 {
        // SAFETY: the handle is either live, or freed and therefore refused by the ABI.
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

    /// Give up a reference. Harmless on a handle already released.
    pub(crate) fn release(self) {
        // SAFETY: the ABI documents a second release as doing nothing.
        unsafe { ak_request_release(self.0) };
    }
}

impl Drop for Request {
    fn drop(&mut self) {
        // Gives up this side's reference and cancels. The context is deliberately not reclaimed
        // here: the driving task still owns it and hands it back on `COMPLETED`.
        // SAFETY: created by `ak_request_start` and released exactly once, here.
        unsafe { ak_request_release(self.raw) };
    }
}
