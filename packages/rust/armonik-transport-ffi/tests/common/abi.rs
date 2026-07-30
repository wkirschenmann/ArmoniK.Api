//! A safe wrapper over the `ak_*` entry points, and the reference for how to use them.
//!
//! Every test drives the real ABI through this module: the same functions, in the same order, with
//! the same ownership rules the .NET side will follow. It exists for two reasons beyond making the
//! tests readable.
//!
//! First, `unsafe` belongs in one reviewed place. A test that scattered raw pointers and
//! `ak_bytes_free` calls through twenty functions would be a poor guarantee of anything.
//!
//! Second, the blob decoding here is written from the *documented* format in
//! `include/armonik_transport_ffi.h`, not by calling back into the crate's own decoder. A round trip
//! through one implementation proves only that it is self-consistent; the .NET side will have its own
//! reader, and this stands in for it.

use std::ffi::c_void;
use std::time::{Duration, Instant};

use armonik_transport_ffi::{
    ak_bytes, ak_bytes_free, ak_bytes_in, ak_call, ak_call_cancel, ak_call_close_send,
    ak_call_free, ak_call_start, ak_call_status, ak_call_try_headers, ak_call_try_recv,
    ak_call_try_send, ak_call_wait_handle, ak_client, ak_client_create, ak_client_free, status,
};

/// How long a helper that waits for the server will wait before declaring the test hung. Generous:
/// it is a backstop against a deadlock, not a performance assertion, and CI runners are slow.
const PATIENCE: Duration = Duration::from_secs(20);

/// The `method_kind` values the ABI accepts, mirroring the header's `AK_METHOD_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Unary = 0,
    ClientStreaming = 1,
    ServerStreaming = 2,
    BidiStreaming = 3,
}

/// What one `ak_call_try_recv` reported.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Poll {
    Pending,
    Message(Vec<u8>),
    Completed,
}

/// A completed call's outcome, as `ak_call_status` reports it.
#[derive(Debug)]
pub(crate) struct Outcome {
    pub(crate) code: i32,
    pub(crate) message: String,
    pub(crate) trailers: Vec<(String, Vec<u8>)>,
}

/// Take ownership of an `ak_bytes` into a `Vec`, freeing it exactly once.
///
/// # Safety
///
/// `bytes` must be a value the crate produced and that has not been freed.
unsafe fn take(bytes: ak_bytes) -> Vec<u8> {
    let copy = if bytes.ptr.is_null() || bytes.len == 0 {
        Vec::new()
    } else {
        // SAFETY: the crate documents `ptr`/`len` as readable until `ak_bytes_free`.
        unsafe { std::slice::from_raw_parts(bytes.ptr, bytes.len) }.to_vec()
    };
    // SAFETY: forwarded from this function's contract; freed exactly once, here.
    unsafe { ak_bytes_free(bytes) };
    copy
}

/// The zeroed `ak_bytes` an out-parameter starts as, and that the ABI uses for "no data".
fn no_bytes() -> ak_bytes {
    ak_bytes {
        ptr: std::ptr::null(),
        len: 0,
        owner: std::ptr::null_mut(),
    }
}

/// Decode a key/value blob, per the format documented in the generated header.
fn decode_blob(blob: &[u8]) -> Vec<(String, Vec<u8>)> {
    if blob.is_empty() {
        return Vec::new();
    }
    let mut cursor = blob;
    let read_u32 = |cursor: &mut &[u8]| -> usize {
        let (head, tail) = cursor.split_at(4);
        *cursor = tail;
        u32::from_ne_bytes(head.try_into().expect("four bytes")) as usize
    };
    let count = read_u32(&mut cursor);
    let mut pairs = Vec::with_capacity(count);
    for _ in 0..count {
        let key_len = read_u32(&mut cursor);
        let (key, tail) = cursor.split_at(key_len);
        cursor = tail;
        let value_len = read_u32(&mut cursor);
        let (value, tail) = cursor.split_at(value_len);
        cursor = tail;
        pairs.push((String::from_utf8_lossy(key).into_owned(), value.to_vec()));
    }
    assert!(cursor.is_empty(), "the blob had trailing bytes");
    pairs
}

/// Encode a key/value blob, the way the .NET side will.
pub(crate) fn encode_blob<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> Vec<u8> {
    let mut body = Vec::new();
    let mut count = 0u32;
    for (key, value) in pairs {
        body.extend_from_slice(&(key.len() as u32).to_ne_bytes());
        body.extend_from_slice(key.as_bytes());
        body.extend_from_slice(&(value.len() as u32).to_ne_bytes());
        body.extend_from_slice(value);
        count += 1;
    }
    let mut blob = count.to_ne_bytes().to_vec();
    blob.extend_from_slice(&body);
    blob
}

/// Wait for the call's event handle, or, where there is no such handle, yield briefly.
///
/// On Windows this exercises the real wake-up path the .NET side uses. Elsewhere
/// `ak_call_wait_handle` returns null by design — the cdylib only ever ships for Windows — and the
/// tests fall back to polling, which tests everything except the signalling itself.
fn wait(handle: *mut c_void) {
    #[cfg(windows)]
    {
        if !handle.is_null() {
            #[link(name = "kernel32")]
            extern "system" {
                fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
            }
            // SAFETY: the handle is borrowed from a live `ak_call`, valid until `ak_call_free`, and
            // waiting on it neither closes nor mutates it. A short timeout rather than `INFINITE`
            // keeps a lost wake-up from turning into a hang instead of a failure.
            unsafe { WaitForSingleObject(handle, 20) };
            return;
        }
    }
    let _ = handle;
    std::thread::sleep(Duration::from_millis(1));
}

/// A connected client, freed on drop.
pub(crate) struct Client {
    raw: *mut ak_client,
}

impl Client {
    /// Connect to `endpoint`, with `options` on top of the defaults.
    pub(crate) fn connect(endpoint: &str, options: &[(&str, &str)]) -> Result<Self, (i32, String)> {
        let mut pairs: Vec<(&str, &[u8])> = vec![("Endpoint", endpoint.as_bytes())];
        pairs.extend(options.iter().map(|(key, value)| (*key, value.as_bytes())));
        let blob = encode_blob(pairs);

        let empty = ak_bytes_in {
            ptr: std::ptr::null(),
            len: 0,
        };
        let mut raw: *mut ak_client = std::ptr::null_mut();
        let mut error = no_bytes();
        // SAFETY: `blob` outlives the call, the three certificate views are empty, and both
        // out-parameters point at live locals.
        let code = unsafe {
            ak_client_create(
                blob.as_ptr(),
                blob.len(),
                empty,
                empty,
                empty,
                std::ptr::addr_of_mut!(raw),
                std::ptr::addr_of_mut!(error),
            )
        };
        // SAFETY: produced by the call above, freed exactly once.
        let message = String::from_utf8_lossy(&unsafe { take(error) }).into_owned();

        if code == status::OK {
            Ok(Self { raw })
        } else {
            Err((code, message))
        }
    }

    /// Connect, asserting success.
    pub(crate) fn to(endpoint: &str, options: &[(&str, &str)]) -> Self {
        Self::connect(endpoint, options).expect("the test server should be reachable")
    }

    /// A retry policy short enough for a test: three attempts, ten-millisecond backoff.
    pub(crate) fn quick_retry() -> Vec<(&'static str, &'static str)> {
        vec![
            ("MaxAttempts", "3"),
            ("InitialBackOff", "10ms"),
            ("MaxBackOff", "20ms"),
        ]
    }

    /// Start a call. `deadline_ms <= 0` means no deadline.
    pub(crate) fn start(&self, path: &str, kind: Kind, options: StartOptions) -> Call {
        self.try_start(path, kind, options)
            .expect("starting a call should succeed")
    }

    pub(crate) fn try_start(
        &self,
        path: &str,
        kind: Kind,
        options: StartOptions,
    ) -> Result<Call, (i32, String)> {
        // SAFETY: `self.raw` is live until `Drop`, which cannot run while `&self` is held.
        unsafe { start_call(self.raw, path, kind, options) }
    }

    /// The raw handle, for the tests that deliberately misuse the ABI.
    pub(crate) fn as_ptr(&self) -> *mut ak_client {
        self.raw
    }

    /// Free the client now and return its (now dangling) handle, for use-after-free tests.
    pub(crate) fn free_early(self) -> *mut ak_client {
        let raw = self.raw;
        // SAFETY: a live handle, freed exactly once — `into_raw`-style, so `Drop` never runs.
        unsafe { ak_client_free(raw) };
        std::mem::forget(self);
        raw
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // SAFETY: produced by `ak_client_create` and freed exactly once, here.
        unsafe { ak_client_free(self.raw) };
    }
}

/// The arguments of [`Client::start`] that usually take their default.
pub(crate) struct StartOptions {
    pub(crate) metadata: Vec<u8>,
    pub(crate) deadline_ms: i64,
    pub(crate) queue_capacity: usize,
}

impl Default for StartOptions {
    fn default() -> Self {
        Self {
            metadata: Vec::new(),
            deadline_ms: 0,
            queue_capacity: 8,
        }
    }
}

impl StartOptions {
    pub(crate) fn deadline(millis: i64) -> Self {
        Self {
            deadline_ms: millis,
            ..Self::default()
        }
    }

    pub(crate) fn metadata<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> Self {
        Self {
            metadata: encode_blob(pairs),
            ..Self::default()
        }
    }

    pub(crate) fn capacity(slots: usize) -> Self {
        Self {
            queue_capacity: slots,
            ..Self::default()
        }
    }
}

/// # Safety
///
/// `client` must be a live client handle, or a value being deliberately tested as invalid.
unsafe fn start_call(
    client: *mut ak_client,
    path: &str,
    kind: Kind,
    options: StartOptions,
) -> Result<Call, (i32, String)> {
    let mut raw: *mut ak_call = std::ptr::null_mut();
    let mut error = no_bytes();
    // SAFETY: `path` and `options.metadata` outlive the call; both out-parameters point at live
    // locals; `client` is forwarded from this function's contract.
    let code = unsafe {
        ak_call_start(
            client,
            path.as_ptr(),
            path.len(),
            kind as i32,
            options.metadata.as_ptr(),
            options.metadata.len(),
            options.deadline_ms,
            options.queue_capacity,
            std::ptr::addr_of_mut!(raw),
            std::ptr::addr_of_mut!(error),
        )
    };
    // SAFETY: produced by the call above, freed exactly once.
    let message = String::from_utf8_lossy(&unsafe { take(error) }).into_owned();

    if code == status::OK {
        Ok(Call { raw })
    } else {
        Err((code, message))
    }
}

/// An in-flight or completed call, freed on drop.
pub(crate) struct Call {
    raw: *mut ak_call,
}

impl Call {
    pub(crate) fn send(&self, payload: &[u8]) -> i32 {
        // SAFETY: a live handle; `payload` outlives the call.
        unsafe { ak_call_try_send(self.raw, payload.as_ptr(), payload.len()) }
    }

    /// Send, asserting the queue had room.
    pub(crate) fn send_ok(&self, payload: &[u8]) {
        assert_eq!(
            self.send(payload),
            status::OK,
            "the send should be accepted"
        );
    }

    pub(crate) fn close_send(&self) -> i32 {
        // SAFETY: a live handle.
        unsafe { ak_call_close_send(self.raw) }
    }

    pub(crate) fn close_send_ok(&self) {
        assert_eq!(self.close_send(), status::OK, "closing send should succeed");
    }

    pub(crate) fn cancel(&self) -> i32 {
        // SAFETY: a live handle.
        unsafe { ak_call_cancel(self.raw) }
    }

    pub(crate) fn wait_handle(&self) -> *mut c_void {
        // SAFETY: a live handle.
        unsafe { ak_call_wait_handle(self.raw) }
    }

    pub(crate) fn try_recv(&self) -> Poll {
        let mut message = no_bytes();
        let mut state = -1i32;
        // SAFETY: a live handle; both out-parameters point at live locals.
        let code = unsafe {
            ak_call_try_recv(
                self.raw,
                std::ptr::addr_of_mut!(message),
                std::ptr::addr_of_mut!(state),
            )
        };
        assert_eq!(code, status::OK, "polling a live call should succeed");
        match state {
            0 => Poll::Pending,
            // SAFETY: state 1 means the crate produced a message; freed exactly once.
            1 => Poll::Message(unsafe { take(message) }),
            2 => Poll::Completed,
            other => panic!("unknown recv state {other}"),
        }
    }

    /// The response headers, or `None` until they arrive.
    pub(crate) fn headers(&self) -> Option<Vec<(String, Vec<u8>)>> {
        let mut blob = no_bytes();
        // SAFETY: a live handle; the out-parameter points at a live local.
        let code = unsafe { ak_call_try_headers(self.raw, std::ptr::addr_of_mut!(blob)) };
        assert_eq!(code, status::OK, "reading headers should succeed");
        let empty = blob.owner.is_null();
        // SAFETY: produced by the call above, freed exactly once.
        let bytes = unsafe { take(blob) };
        // An empty `ak_bytes` means "not yet"; an encoded but empty map is four bytes of count.
        (!empty).then(|| decode_blob(&bytes))
    }

    /// The final status, or `None` while the call is still running.
    pub(crate) fn status(&self) -> Option<Outcome> {
        let mut code = 0i32;
        let mut message = no_bytes();
        let mut trailers = no_bytes();
        // SAFETY: a live handle; all three out-parameters point at live locals.
        let outcome = unsafe {
            ak_call_status(
                self.raw,
                std::ptr::addr_of_mut!(code),
                std::ptr::addr_of_mut!(message),
                std::ptr::addr_of_mut!(trailers),
            )
        };
        if outcome == status::INVALID_STATE {
            return None;
        }
        assert_eq!(outcome, status::OK, "reading a status should succeed");
        // SAFETY: both produced by the call above, each freed exactly once.
        let message = String::from_utf8_lossy(&unsafe { take(message) }).into_owned();
        let trailers = decode_blob(&unsafe { take(trailers) });
        Some(Outcome {
            code,
            message,
            trailers,
        })
    }

    /// Drain until the call completes, returning every message in order and the final status.
    ///
    /// This is the loop the .NET side runs: poll until nothing is left, wait on the handle, repeat.
    pub(crate) fn drain(&self) -> (Vec<Vec<u8>>, Outcome) {
        let mut messages = Vec::new();
        let started = Instant::now();
        loop {
            match self.try_recv() {
                Poll::Message(message) => messages.push(message),
                Poll::Completed => {
                    let outcome = self
                        .status()
                        .expect("a completed call must have published its status");
                    return (messages, outcome);
                }
                Poll::Pending => {
                    assert!(
                        started.elapsed() < PATIENCE,
                        "the call never completed: {} message(s) received so far",
                        messages.len()
                    );
                    wait(self.wait_handle());
                }
            }
        }
    }

    /// Wait until `predicate` holds, failing the test rather than spinning forever.
    pub(crate) fn wait_until(&self, what: &str, mut predicate: impl FnMut(&Self) -> bool) {
        let started = Instant::now();
        while !predicate(self) {
            assert!(started.elapsed() < PATIENCE, "timed out waiting for {what}");
            wait(self.wait_handle());
        }
    }

    /// The raw handle, for the tests that deliberately misuse the ABI.
    pub(crate) fn as_ptr(&self) -> *mut ak_call {
        self.raw
    }

    /// Free the call now and return its (now dangling) handle, for use-after-free tests.
    pub(crate) fn free_early(self) -> *mut ak_call {
        let raw = self.raw;
        // SAFETY: a live handle, freed exactly once; `Drop` never runs thanks to the `forget`.
        unsafe { ak_call_free(raw) };
        std::mem::forget(self);
        raw
    }
}

impl Drop for Call {
    fn drop(&mut self) {
        // SAFETY: produced by `ak_call_start` and freed exactly once, here.
        unsafe { ak_call_free(self.raw) };
    }
}
