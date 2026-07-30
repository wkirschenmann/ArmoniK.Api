//! Driving one RPC call.
//!
//! [`ak_call_start`] spawns a background task on the shared runtime and returns immediately; every
//! other function here is a synchronous, non-blocking poll of state that task publishes. Nothing
//! ever calls back into managed code: messages and the final status travel through bounded
//! `tokio::sync::mpsc` channels that [`ak_call_try_recv`]/[`ak_call_status`] drain, and the caller
//! is woken up through an OS event this crate creates and owns, borrowed to the caller by
//! [`ak_call_wait_handle`] — see [`crate::event`] for why Rust rather than .NET owns it.
//!
//! # Which calls can be retried
//!
//! Only [`MethodKind::Unary`] and [`MethodKind::ServerStreaming`] ever retry, and only while zero
//! response messages have been produced — see [`armonik_transport::RetryPolicy::may_replay`]. Both
//! shapes send exactly one request message, so this module buffers it (waiting for
//! [`ak_call_close_send`]) before making the first attempt, which is what makes replaying it
//! possible. [`MethodKind::ClientStreaming`] and [`MethodKind::BidiStreaming`] are never retried and
//! are streamed straight through as they arrive, with no buffering.
//!
//! This module does not reuse [`armonik_transport::retry_with`]: that helper is built around "one
//! attempt produces one `Result<T, Status>`", whereas retrying here has to interleave with
//! forwarding messages as they arrive and deciding, mid-stream, whether the *shape* of what has
//! happened so far still allows a replay. It calls the same, already-tested
//! [`armonik_transport::RetryPolicy::should_retry`] that `retry_with` is itself built on.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;

use armonik_transport::reexports::http::uri::PathAndQuery;
use armonik_transport::reexports::tokio_stream::Stream;
use armonik_transport::reexports::tonic::client::Grpc;
use armonik_transport::reexports::tonic::metadata::MetadataMap;
use armonik_transport::reexports::tonic::transport::Channel;
use armonik_transport::reexports::tonic::{Code, Request, Status};
use armonik_transport::MethodKind;
use armonik_transport::RetryPolicy;
use bytes::Bytes;
use tokio::sync::{mpsc, Notify};

use crate::codec::BytesCodec;
use crate::error::ak_bytes;
use crate::event::OwnedEvent;
use crate::handle::LiveSet;

static LIVE: std::sync::OnceLock<LiveSet> = std::sync::OnceLock::new();

fn live() -> &'static LiveSet {
    LIVE.get_or_init(LiveSet::new)
}

/// An opaque, in-flight or completed RPC call.
///
/// Obtained from [`ak_call_start`], released with [`ak_call_free`]. Every function here that takes
/// `*const ak_call` only ever reads through a shared reference: the mutable state
/// (`send`/`recv`/`headers`/`status`) is behind its own `Mutex`, none of them held across an
/// `.await`, so concurrent calls into this API for the same handle (e.g. a reader thread polling
/// `ak_call_try_recv` while a writer thread polls `ak_call_try_send`) are safe, though calling the
/// same function concurrently with itself on the same handle is not a scenario this crate
/// serialises against and the caller should not do it.
pub struct ak_call {
    send: Mutex<Option<mpsc::Sender<Bytes>>>,
    recv: Mutex<mpsc::Receiver<Bytes>>,
    headers: Arc<Mutex<Option<MetadataMap>>>,
    status: Arc<Mutex<Option<StatusRecord>>>,
    cancel: Arc<Cancellation>,
    /// The event the caller waits on, created and owned by this crate. The driving task holds a
    /// clone, so the handle outlives whichever of the two finishes last and is never signalled
    /// after being closed — see [`crate::event`].
    event: Arc<OwnedEvent>,
    /// Kept so `ak_call_free` can forcibly stop the driving task if it is still running rather than
    /// leaving it to notice cancellation on its own time.
    task: tokio::task::JoinHandle<()>,
}

/// A one-shot, idempotent cancellation signal shared between the FFI thread and the driving task.
struct Cancellation {
    flag: AtomicBool,
    notify: Notify,
}

impl Cancellation {
    fn new() -> Self {
        Self {
            flag: AtomicBool::new(false),
            notify: Notify::new(),
        }
    }

    /// Ask the driving task to stop. Safe to call more than once; only the first call has an
    /// effect.
    fn cancel(&self) {
        self.flag.store(true, Ordering::Release);
        self.notify.notify_one();
    }

    fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }

    /// Resolve once [`Self::cancel`] has been called.
    ///
    /// `Notify::notify_one` stores a permit if nobody is waiting yet, so a `cancel()` that races
    /// ahead of the driving task reaching this await is not lost.
    async fn cancelled(&self) {
        self.notify.notified().await;
    }
}

/// The outcome of a completed call, as `ak_call_status` reports it.
///
/// `message` is a [`Bytes`], not a `String`: `ak_call_status` may be read more than once (the
/// contract does not say "only once", unlike `ak_call_try_recv`'s draining semantics), so the
/// message has to survive being handed out repeatedly. Cloning a `Bytes` is a refcount bump, not a
/// copy of the text, so repeated reads stay cheap regardless.
struct StatusRecord {
    code: i32,
    message: Bytes,
    trailers: MetadataMap,
}

impl StatusRecord {
    fn ok(trailers: MetadataMap) -> Self {
        Self {
            code: Code::Ok as i32,
            message: Bytes::new(),
            trailers,
        }
    }

    fn from_status(mut status: Status) -> Self {
        Self {
            code: crate::status::from_grpc(status.code()),
            // `Status::message` only ever hands out a borrow, so this copy — turning it into
            // something this crate can own and hand out repeatedly — is unavoidable, but it only
            // ever happens once per call rather than once per `ak_call_status` read.
            message: Bytes::copy_from_slice(status.message().as_bytes()),
            // Moved out of `status` (which is about to be dropped) rather than cloned: `status` is
            // owned here, so there is nothing left needing the original afterwards.
            trailers: std::mem::take(status.metadata_mut()),
        }
    }

    fn cancelled() -> Self {
        Self {
            code: Code::Cancelled as i32,
            message: Bytes::from_static(b"the call was cancelled"),
            trailers: MetadataMap::new(),
        }
    }

    /// The caller stopped reading (freed the call, or its receive queue was dropped) before the
    /// call completed on its own.
    fn abandoned() -> Self {
        Self {
            code: Code::Cancelled as i32,
            message: Bytes::from_static(b"the call was abandoned by the caller"),
            trailers: MetadataMap::new(),
        }
    }

    fn deadline_exceeded() -> Self {
        Self {
            code: Code::DeadlineExceeded as i32,
            message: Bytes::from_static(b"the deadline was exceeded"),
            trailers: MetadataMap::new(),
        }
    }

    /// The driving task failed in a way that is a bug in this crate, `reason` being whatever could be
    /// said about it.
    ///
    /// The message matters as much as the code: this is what a bug report will be written from, and
    /// "something went wrong" is not a bug report. A panic's own text — `` `send_item` called without
    /// first calling `poll_reserve` ``, say — is the whole diagnosis, and it has nowhere else to go
    /// once the task that produced it is gone.
    fn internal_failure(reason: &str) -> Self {
        Self {
            code: crate::status::INTERNAL_PANIC,
            message: Bytes::from(format!(
                "the call failed inside the native transport: {reason}"
            )),
            trailers: MetadataMap::new(),
        }
    }
}

/// Owns publishing the final status, and guarantees something is published.
///
/// [`ak_call_try_recv`] reports a call as completed as soon as the response channel disconnects,
/// which happens when the driving task's future is dropped — on a normal return, but equally on a
/// panic or a `JoinHandle::abort`. Without this guard, those cases leave a call that reports
/// "completed" while [`ak_call_status`] still answers `INVALID_STATE`, and there is no state left for
/// the caller to wait for: a hang, from a bug that would otherwise be invisible until production.
///
/// So the invariant is enforced by construction rather than by discipline: whatever happens to the
/// task, dropping this either finds a status already published or publishes one saying it did not.
///
/// This is the *last* net, not the first one. A panic inside the work itself is caught by
/// [`crate::guard::catch_unwind_future`], which can still say what the panic was; by the time this
/// `Drop` runs there is nothing left to report but the fact.
struct StatusPublisher {
    slot: Arc<Mutex<Option<StatusRecord>>>,
    event: Arc<OwnedEvent>,
}

impl StatusPublisher {
    fn new(slot: Arc<Mutex<Option<StatusRecord>>>, event: Arc<OwnedEvent>) -> Self {
        Self { slot, event }
    }

    /// Publish `record` unless something already has.
    fn publish(&self, record: StatusRecord) {
        let mut slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.is_none() {
            *slot = Some(record);
        }
        drop(slot);
        self.event.signal();
    }
}

impl Drop for StatusPublisher {
    fn drop(&mut self) {
        self.publish(StatusRecord::internal_failure(
            "its driving task ended without producing a status",
        ));
    }
}

/// Wait until the channel will accept a new request.
///
/// `Grpc::streaming` does not do this itself, and every `tonic`-generated client calls it first, with
/// this same mapping to `Unknown` — which is in the default retryable set, so a channel that has just
/// lost its connection is retried rather than surfaced. Skipping it trips `tower::Buffer`'s
/// "send_item called without first calling poll_reserve" assertion instead of returning an error.
async fn ready(grpc: &mut Grpc<Channel>) -> Result<(), Status> {
    grpc.ready()
        .await
        .map_err(|error| Status::unknown(format!("Service was not ready: {error}")))
}

/// Feeds request messages from a bounded channel into `tonic` as a `Stream`, signalling the event
/// handle every time a slot frees up so a blocked `ak_call_try_send` knows to retry.
///
/// Hand-rolled rather than depending on `tokio-stream`'s `ReceiverStream` (which needs its `sync`
/// feature, pulling in `tokio-util` transitively): the whole thing is a two-line forward to
/// `Receiver::poll_recv`.
struct RequestStream {
    rx: mpsc::Receiver<Bytes>,
    /// Owned rather than borrowed: this stream is moved into the `tonic` request and outlives the
    /// stack frame it was built in.
    event: Arc<OwnedEvent>,
}

impl Stream for RequestStream {
    type Item = Bytes;

    fn poll_next(mut self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Bytes>> {
        let poll = self.rx.poll_recv(cx);
        if matches!(poll, Poll::Ready(Some(_))) {
            self.event.signal();
        }
        poll
    }
}

/// Whether `kind` sends exactly one request message and so may buffer it for a retry.
fn is_retryable_shape(kind: MethodKind) -> bool {
    matches!(kind, MethodKind::Unary | MethodKind::ServerStreaming)
}

use std::future::Future;

/// Await `body`, bounded by an absolute `deadline` when one is set. [`None`] means it expired.
///
/// Absolute rather than a duration: a gRPC deadline covers the whole call, so every phase — waiting
/// for the request messages, each attempt, the backoff between attempts — has to draw from the same
/// budget. Passing a `Duration` down instead would restart the clock at each phase and let a call
/// outlive its deadline several times over.
async fn with_optional_deadline<T>(
    deadline: Option<tokio::time::Instant>,
    body: impl Future<Output = T>,
) -> Option<T> {
    match deadline {
        Some(deadline) => tokio::time::timeout_at(deadline, body).await.ok(),
        None => Some(body.await),
    }
}

/// The whole background task: pick the retryable or the plain streaming path, run it to
/// completion, then publish the final status and let both channels disconnect.
#[allow(clippy::too_many_arguments)]
async fn drive(
    grpc: Grpc<Channel>,
    retry: Option<RetryPolicy>,
    path: PathAndQuery,
    kind: MethodKind,
    metadata: MetadataMap,
    deadline: Option<Duration>,
    request_rx: mpsc::Receiver<Bytes>,
    response_tx: mpsc::Sender<Bytes>,
    headers_slot: Arc<Mutex<Option<MetadataMap>>>,
    status_slot: Arc<Mutex<Option<StatusRecord>>>,
    cancel: Arc<Cancellation>,
    event: &Arc<OwnedEvent>,
) {
    // Taken before anything can fail, so the guarantee covers the whole body below.
    let publisher = StatusPublisher::new(status_slot, Arc::clone(event));

    // Anchored once, here, so every phase below draws from the same budget.
    let deadline = deadline.map(|deadline| tokio::time::Instant::now() + deadline);

    // Boxed and run through `catch_unwind_future` so a panic in here becomes an outcome the caller
    // can read instead of a call that reports itself finished with nothing to say. `response_tx`
    // deliberately stays *outside* this future: the caller sees a call as completed the moment that
    // sender drops, so it has to outlive the publish below.
    let body: std::pin::Pin<Box<dyn Future<Output = StatusRecord> + Send + '_>> =
        Box::pin(drive_to_outcome(
            grpc,
            retry,
            path,
            kind,
            metadata,
            deadline,
            request_rx,
            &response_tx,
            headers_slot,
            cancel,
            event,
        ));

    publisher.publish(outcome_of(body).await);
    // `response_tx` drops here (and, on the retryable path, nothing else was holding the sender
    // half of the request channel either), disconnecting the channel `ak_call_try_recv` polls —
    // that disconnection is exactly what it reports as "Completed". Publishing first is what makes
    // "completed implies a status is readable" true.
}

/// Run `body`, turning a panic inside it into the outcome the caller will read.
///
/// Named, rather than inlined into [`drive`], so a test can exercise the panic path through the same
/// code the real call goes through: this is the one place that decides what a bug in this crate looks
/// like from .NET.
async fn outcome_of(
    body: std::pin::Pin<Box<dyn Future<Output = StatusRecord> + Send + '_>>,
) -> StatusRecord {
    match crate::guard::catch_unwind_future(body).await {
        Ok(outcome) => outcome,
        Err(message) => StatusRecord::internal_failure(&message),
    }
}

/// Everything that decides the call's outcome, with nothing to publish it.
///
/// Split out of [`drive`] purely so it can be run under [`crate::guard::catch_unwind_future`]: a
/// panic in here has to become a status, and that means the code that publishes the status cannot be
/// the code that might panic.
#[allow(clippy::too_many_arguments)]
async fn drive_to_outcome(
    grpc: Grpc<Channel>,
    retry: Option<RetryPolicy>,
    path: PathAndQuery,
    kind: MethodKind,
    metadata: MetadataMap,
    deadline: Option<tokio::time::Instant>,
    mut request_rx: mpsc::Receiver<Bytes>,
    response_tx: &mpsc::Sender<Bytes>,
    headers_slot: Arc<Mutex<Option<MetadataMap>>>,
    cancel: Arc<Cancellation>,
    event: &Arc<OwnedEvent>,
) -> StatusRecord {
    if is_retryable_shape(kind) {
        // The one request message has to be buffered before the first attempt, since replaying a
        // call means resending it. The wait for `ak_call_close_send` is itself covered by the
        // cancellation signal and the deadline: without that, a caller that forgot to close its
        // send side would leave this task parked here for the life of the process, and
        // `ak_call_cancel` would have no effect until a message happened to arrive.
        let buffering = async {
            let mut buffered = Vec::new();
            while let Some(message) = request_rx.recv().await {
                buffered.push(message);
                event.signal();
            }
            buffered
        };

        // `Some(None)` is a deadline that expired while buffering, `None` a cancellation: the two
        // have to stay distinguishable, since they report different statuses.
        let buffered = tokio::select! {
            biased;
            () = cancel.cancelled() => None,
            buffered = with_optional_deadline(deadline, buffering) => Some(buffered),
        };

        match buffered {
            None => StatusRecord::cancelled(),
            Some(None) => StatusRecord::deadline_exceeded(),
            Some(Some(buffered)) => {
                drive_retryable(
                    grpc,
                    retry,
                    &path,
                    kind,
                    metadata,
                    deadline,
                    buffered,
                    response_tx,
                    &headers_slot,
                    &cancel,
                    event,
                )
                .await
            }
        }
    } else {
        // Never retried, so there is only ever one attempt: `metadata` moves in directly, with no
        // clone at all.
        drive_streaming(
            grpc,
            &path,
            metadata,
            deadline,
            request_rx,
            response_tx,
            &headers_slot,
            &cancel,
            event,
        )
        .await
    }
}

enum AttemptOutcome {
    Success(MetadataMap),
    CallerGone,
}

#[allow(clippy::too_many_arguments)]
async fn drive_retryable(
    mut grpc: Grpc<Channel>,
    retry: Option<RetryPolicy>,
    path: &PathAndQuery,
    kind: MethodKind,
    mut metadata: MetadataMap,
    deadline: Option<tokio::time::Instant>,
    mut buffered: Vec<Bytes>,
    response_tx: &mpsc::Sender<Bytes>,
    headers_slot: &Mutex<Option<MetadataMap>>,
    cancel: &Cancellation,
    event: &Arc<OwnedEvent>,
) -> StatusRecord {
    let mut attempts_made = 0u32;

    loop {
        attempts_made += 1;
        if cancel.is_cancelled() {
            return StatusRecord::cancelled();
        }

        // Without a retry policy, `should_retry` below can never fire, so this is always the only
        // attempt: `mem::take` moves `metadata`/`buffered` in directly rather than cloning them for
        // a retry that will never happen. With a policy, each attempt needs its own owned copies —
        // cheap regardless, since neither clone touches message payload bytes: `MetadataMap`
        // holds a handful of small headers, and cloning a `Vec<Bytes>` only bumps refcounts, it
        // never copies what each `Bytes` points at.
        let (attempt_metadata, attempt_messages) = if retry.is_some() {
            (metadata.clone(), buffered.clone())
        } else {
            (std::mem::take(&mut metadata), std::mem::take(&mut buffered))
        };

        // `grpc` is reused across attempts rather than re-cloned each time: `Grpc::streaming` takes
        // `&mut self` and does not need a fresh clone between sequential calls, so cloning it here
        // would only be an unnecessary atomic refcount bump on the underlying channel.
        let attempt_body = async {
            if let Err(status) = ready(&mut grpc).await {
                return Err((status, 0u64));
            }

            let mut request = Request::new(armonik_transport::reexports::tokio_stream::iter(
                attempt_messages,
            ));
            *request.metadata_mut() = attempt_metadata;

            let response = match grpc.streaming(request, path.clone(), BytesCodec).await {
                Ok(response) => response,
                Err(status) => return Err((status, 0u64)),
            };
            let (headers, mut stream, _extensions) = response.into_parts();
            *headers_slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(headers);
            event.signal();

            let mut messages_received = 0u64;
            loop {
                match stream.message().await {
                    Ok(Some(message)) => {
                        messages_received += 1;
                        if response_tx.send(message).await.is_err() {
                            return Ok(AttemptOutcome::CallerGone);
                        }
                        event.signal();
                    }
                    Ok(None) => {
                        let trailers = stream.trailers().await.ok().flatten().unwrap_or_default();
                        return Ok(AttemptOutcome::Success(trailers));
                    }
                    Err(status) => return Err((status, messages_received)),
                }
            }
        };

        let raced = tokio::select! {
            biased;
            () = cancel.cancelled() => None,
            outcome = with_optional_deadline(deadline, attempt_body) => Some(outcome),
        };

        match raced {
            None => return StatusRecord::cancelled(),
            Some(None) => return StatusRecord::deadline_exceeded(),
            Some(Some(Ok(AttemptOutcome::Success(trailers)))) => return StatusRecord::ok(trailers),
            Some(Some(Ok(AttemptOutcome::CallerGone))) => return StatusRecord::abandoned(),
            Some(Some(Err((status, messages_received)))) => {
                let decision = retry.as_ref().and_then(|policy| {
                    policy.should_retry(kind, attempts_made, messages_received, status.code())
                });

                let Some(delay) = decision else {
                    return StatusRecord::from_status(status);
                };

                tracing::debug!(
                    attempts_made,
                    code = ?status.code(),
                    delay = ?delay,
                    "Retrying a call from the FFI layer"
                );

                let waited = tokio::select! {
                    biased;
                    () = cancel.cancelled() => false,
                    () = tokio::time::sleep(delay) => true,
                };
                if !waited {
                    return StatusRecord::cancelled();
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn drive_streaming(
    mut grpc: Grpc<Channel>,
    path: &PathAndQuery,
    metadata: MetadataMap,
    deadline: Option<tokio::time::Instant>,
    request_rx: mpsc::Receiver<Bytes>,
    response_tx: &mpsc::Sender<Bytes>,
    headers_slot: &Mutex<Option<MetadataMap>>,
    cancel: &Cancellation,
    event: &Arc<OwnedEvent>,
) -> StatusRecord {
    // This shape is never retried, so there is only ever one attempt: `metadata` moves straight
    // into the request, with no clone at all.
    let body = async {
        if let Err(status) = ready(&mut grpc).await {
            return StatusRecord::from_status(status);
        }

        let mut request = Request::new(RequestStream {
            rx: request_rx,
            // The stream outlives this frame, so it needs its own handle on the event; cloning an
            // `Arc` here is a refcount bump, not a duplicated OS handle.
            event: Arc::clone(event),
        });
        *request.metadata_mut() = metadata;

        let response = match grpc.streaming(request, path.clone(), BytesCodec).await {
            Ok(response) => response,
            Err(status) => return StatusRecord::from_status(status),
        };
        let (headers, mut stream, _extensions) = response.into_parts();
        *headers_slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(headers);
        event.signal();

        loop {
            match stream.message().await {
                Ok(Some(message)) => {
                    if response_tx.send(message).await.is_err() {
                        return StatusRecord::abandoned();
                    }
                    event.signal();
                }
                Ok(None) => {
                    let trailers = stream.trailers().await.ok().flatten().unwrap_or_default();
                    return StatusRecord::ok(trailers);
                }
                Err(status) => return StatusRecord::from_status(status),
            }
        }
    };

    tokio::select! {
        biased;
        () = cancel.cancelled() => StatusRecord::cancelled(),
        outcome = with_optional_deadline(deadline, body) => outcome.unwrap_or_else(StatusRecord::deadline_exceeded),
    }
}

fn method_kind_from_i32(value: i32) -> Option<MethodKind> {
    match value {
        0 => Some(MethodKind::Unary),
        1 => Some(MethodKind::ClientStreaming),
        2 => Some(MethodKind::ServerStreaming),
        3 => Some(MethodKind::BidiStreaming),
        _ => None,
    }
}

/// Start a call.
///
/// `deadline_ms <= 0` means no deadline; otherwise the deadline covers the whole call — waiting for
/// the request messages, every attempt, and the backoff between attempts all draw from it.
/// `queue_capacity` bounds the outgoing *and* the incoming message queue independently (each gets
/// that many slots) and is clamped to at least 1.
///
/// On success, `*out` receives a handle that must later be released with [`ak_call_free`]. Wait for
/// new state on the handle from [`ak_call_wait_handle`].
///
/// # Safety
///
/// `client` must be a live handle from [`crate::client::ak_client_create`]. `method_path` must be
/// valid for `method_path_len` UTF-8 bytes. `metadata`, if non-null, must be valid for
/// `metadata_len` bytes and follow the format documented in the generated header. `out` must be
/// non-null and writable, and `out_err`, if non-null, must be writable.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn ak_call_start(
    client: *const crate::client::ak_client,
    method_path: *const u8,
    method_path_len: usize,
    method_kind: i32,
    metadata: *const u8,
    metadata_len: usize,
    deadline_ms: i64,
    queue_capacity: usize,
    out: *mut *mut ak_call,
    out_err: *mut ak_bytes,
) -> i32 {
    crate::guard::catch_unwind_status(out_err, || {
        if client.is_null() {
            return crate::error::FfiError::NullArgument("client").into_ffi_result(out_err);
        }
        if out.is_null() {
            return crate::error::FfiError::NullArgument("out").into_ffi_result(out_err);
        }
        if !crate::client::is_live(client) {
            return crate::error::FfiError::InvalidHandle.into_ffi_result(out_err);
        }

        let Some(kind) = method_kind_from_i32(method_kind) else {
            return crate::error::FfiError::InvalidState("unknown method_kind")
                .into_ffi_result(out_err);
        };

        // SAFETY: forwarded from this function's own contract.
        let path_bytes = unsafe {
            if method_path.is_null() || method_path_len == 0 {
                &[][..]
            } else {
                std::slice::from_raw_parts(method_path, method_path_len)
            }
        };
        let path = match std::str::from_utf8(path_bytes)
            .ok()
            .and_then(|s| PathAndQuery::try_from(s).ok())
        {
            Some(path) => path,
            None => {
                return crate::error::FfiError::InvalidState("method_path is not a valid path")
                    .into_ffi_result(out_err)
            }
        };

        // SAFETY: forwarded from this function's own contract.
        let metadata = match unsafe { crate::metadata::decode(metadata, metadata_len) } {
            Ok(metadata) => metadata,
            Err(error) => return error.into_ffi_result(out_err),
        };

        let deadline = (deadline_ms > 0).then(|| Duration::from_millis(deadline_ms as u64));
        let capacity = queue_capacity.max(1);

        let event = match OwnedEvent::new() {
            Ok(event) => event,
            Err(source) => {
                return crate::error::FfiError::EventCreation(source.to_string())
                    .into_ffi_result(out_err)
            }
        };

        // SAFETY: `client` was checked live above; the whole point of cloning `grpc`/`retry` here,
        // while still holding a validated reference, is that the spawned task below no longer
        // depends on `client` staying alive.
        let (grpc, retry) = unsafe { ((*client).grpc.clone(), (*client).retry.clone()) };

        let (send_tx, send_rx) = mpsc::channel(capacity);
        let (recv_tx, recv_rx) = mpsc::channel(capacity);
        let headers_slot = Arc::new(Mutex::new(None));
        let status_slot = Arc::new(Mutex::new(None));
        let cancel = Arc::new(Cancellation::new());

        // Each of these is cloned before the task takes ownership of its copy, so the handle built
        // below keeps its own. For the event in particular that is what guarantees the OS handle
        // outlives whichever of the task and `ak_call_free` finishes last.
        let task_event = Arc::clone(&event);
        let task_headers = Arc::clone(&headers_slot);
        let task_status = Arc::clone(&status_slot);
        let task_cancel = Arc::clone(&cancel);
        let task = crate::runtime::handle().spawn(async move {
            drive(
                grpc,
                retry,
                path,
                kind,
                metadata,
                deadline,
                send_rx,
                recv_tx,
                task_headers,
                task_status,
                task_cancel,
                &task_event,
            )
            .await;
        });

        let call = ak_call {
            send: Mutex::new(Some(send_tx)),
            recv: Mutex::new(recv_rx),
            headers: headers_slot,
            status: status_slot,
            cancel,
            event,
            task,
        };
        let ptr = Box::into_raw(Box::new(call));
        live().insert(ptr);
        // SAFETY: `out` was checked non-null above.
        unsafe { *out = ptr };
        crate::status::OK
    })
}

/// The OS handle to wait on for new state on this call: a message arrived, the response headers or
/// the final status became available, or a send slot freed up.
///
/// The handle is **borrowed**. It is created and owned by this crate, valid for exactly as long as
/// `call` itself — that is, until [`ak_call_free`] — and must never be closed by the caller. On
/// .NET, wrap it as `new SafeWaitHandle(handle, ownsHandle: false)` to say so in the type system.
///
/// It is an auto-reset event, so each wait that succeeds consumes one signal. A wake-up means "poll
/// again", never "exactly one thing changed": always drain [`ak_call_try_recv`] until it reports
/// pending or completed rather than assuming one signal maps to one message.
///
/// Returns null if `call` is null or not a live handle.
///
/// # Safety
///
/// `call` must be a live handle from [`ak_call_start`], or null.
#[no_mangle]
pub unsafe extern "C" fn ak_call_wait_handle(call: *const ak_call) -> *mut std::ffi::c_void {
    crate::guard::catch_unwind_or(std::ptr::null_mut(), || {
        if call.is_null() || !live().contains(call) {
            return std::ptr::null_mut();
        }
        // SAFETY: checked live above; this crate never moves or invalidates an `ak_call` in place.
        unsafe { &*call }.event.raw()
    })
}

/// Poll for the next incoming message.
///
/// `*out_state` is set to `0` (pending, nothing new), `1` (`*out_msg` holds a message, to be
/// released with [`crate::error::ak_bytes_free`]) or `2` (the call has completed; call
/// [`ak_call_status`] for the outcome).
///
/// # Safety
///
/// `call` must be a live handle. `out_msg` and `out_state` must be non-null and writable.
#[no_mangle]
pub unsafe extern "C" fn ak_call_try_recv(
    call: *const ak_call,
    out_msg: *mut ak_bytes,
    out_state: *mut i32,
) -> i32 {
    crate::guard::catch_unwind_status_only(|| {
        if call.is_null() || out_msg.is_null() || out_state.is_null() {
            return crate::status::NULL_ARGUMENT;
        }
        if !live().contains(call) {
            return crate::status::INVALID_HANDLE;
        }
        // SAFETY: checked live above; this crate never moves or invalidates an `ak_call` in place.
        let call_ref = unsafe { &*call };
        let mut recv = call_ref.recv.lock().unwrap_or_else(PoisonError::into_inner);

        match recv.try_recv() {
            Ok(message) => {
                // `message` is already an owned, decoded `Bytes` sitting in this queue; handing it
                // to `from_bytes` moves it across the ABI directly, with no copy of the payload.
                // SAFETY: `out_msg`/`out_state` checked non-null above.
                unsafe {
                    *out_msg = ak_bytes::from_bytes(message);
                    *out_state = 1;
                }
            }
            Err(mpsc::error::TryRecvError::Empty) => {
                // SAFETY: as above.
                unsafe { *out_state = 0 };
            }
            Err(mpsc::error::TryRecvError::Disconnected) => {
                // SAFETY: as above.
                unsafe { *out_state = 2 };
            }
        }
        crate::status::OK
    })
}

/// Poll for the response headers, once available.
///
/// `*out_headers` is left as the empty [`ak_bytes`] until the headers arrive, and holds them
/// (encoded per `include/armonik_transport_ffi.h`) on every call after that; this does not consume them, so it is
/// safe to call more than once.
///
/// # Safety
///
/// `call` must be a live handle. `out_headers` must be non-null and writable.
#[no_mangle]
pub unsafe extern "C" fn ak_call_try_headers(
    call: *const ak_call,
    out_headers: *mut ak_bytes,
) -> i32 {
    crate::guard::catch_unwind_status_only(|| {
        if call.is_null() || out_headers.is_null() {
            return crate::status::NULL_ARGUMENT;
        }
        if !live().contains(call) {
            return crate::status::INVALID_HANDLE;
        }
        // SAFETY: checked live above.
        let call_ref = unsafe { &*call };
        let headers = call_ref
            .headers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let encoded = match headers.as_ref() {
            Some(map) => match crate::metadata::encode(map) {
                Ok(encoded) => encoded,
                Err(error) => return error.status(),
            },
            None => ak_bytes::EMPTY,
        };
        // SAFETY: `out_headers` checked non-null above.
        unsafe { *out_headers = encoded };
        crate::status::OK
    })
}

/// Send one request message.
///
/// Returns [`crate::status::WOULD_BLOCK`] when the send queue is full; the caller should wait for
/// the event handle and retry. Returns [`crate::status::INVALID_STATE`] once
/// [`ak_call_close_send`] has been called, or once the call has otherwise finished consuming
/// requests.
///
/// # Safety
///
/// `call` must be a live handle. `data` must be valid for `len` bytes, or both null/0.
#[no_mangle]
pub unsafe extern "C" fn ak_call_try_send(
    call: *const ak_call,
    data: *const u8,
    len: usize,
) -> i32 {
    crate::guard::catch_unwind_status_only(|| {
        if call.is_null() {
            return crate::status::NULL_ARGUMENT;
        }
        if !live().contains(call) {
            return crate::status::INVALID_HANDLE;
        }
        // SAFETY: checked live above.
        let call_ref = unsafe { &*call };
        // SAFETY: forwarded from this function's own contract.
        let bytes = unsafe {
            if data.is_null() || len == 0 {
                Bytes::new()
            } else {
                Bytes::copy_from_slice(std::slice::from_raw_parts(data, len))
            }
        };

        let mut guard = call_ref.send.lock().unwrap_or_else(PoisonError::into_inner);
        match guard.as_ref() {
            None => crate::status::INVALID_STATE,
            Some(sender) => match sender.try_send(bytes) {
                Ok(()) => crate::status::OK,
                Err(mpsc::error::TrySendError::Full(_)) => crate::status::WOULD_BLOCK,
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    *guard = None;
                    crate::status::INVALID_STATE
                }
            },
        }
    })
}

/// Signal that no more request messages will be sent.
///
/// # Safety
///
/// `call` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn ak_call_close_send(call: *const ak_call) -> i32 {
    crate::guard::catch_unwind_status_only(|| {
        if call.is_null() {
            return crate::status::NULL_ARGUMENT;
        }
        if !live().contains(call) {
            return crate::status::INVALID_HANDLE;
        }
        // SAFETY: checked live above.
        let call_ref = unsafe { &*call };
        let mut guard = call_ref.send.lock().unwrap_or_else(PoisonError::into_inner);
        match guard.take() {
            Some(_sender) => crate::status::OK,
            None => crate::status::INVALID_STATE,
        }
    })
}

/// Read the final status. Only meaningful once [`ak_call_try_recv`] has reported the call
/// completed; returns [`crate::status::INVALID_STATE`] before that.
///
/// `*out_code` follows the same convention as this crate's own return codes: `0` for success, a
/// positive gRPC status code when the call failed on the wire, and one of the negative
/// `crate::status` values when it failed locally instead — so a caller mapping it to a gRPC status
/// enumeration has to handle the negative case rather than casting blindly.
///
/// # Safety
///
/// `call` must be a live handle. `out_code`, `out_msg` and `out_trailers` must be non-null and
/// writable.
#[no_mangle]
pub unsafe extern "C" fn ak_call_status(
    call: *const ak_call,
    out_code: *mut i32,
    out_msg: *mut ak_bytes,
    out_trailers: *mut ak_bytes,
) -> i32 {
    crate::guard::catch_unwind_status_only(|| {
        if call.is_null() || out_code.is_null() || out_msg.is_null() || out_trailers.is_null() {
            return crate::status::NULL_ARGUMENT;
        }
        if !live().contains(call) {
            return crate::status::INVALID_HANDLE;
        }
        // SAFETY: checked live above.
        let call_ref = unsafe { &*call };
        let guard = call_ref
            .status
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match guard.as_ref() {
            None => crate::status::INVALID_STATE,
            Some(record) => {
                // Encoded before anything is written out, so a failure here leaves every
                // out-parameter untouched rather than half-filled.
                let trailers = match crate::metadata::encode(&record.trailers) {
                    Ok(trailers) => trailers,
                    Err(error) => return error.status(),
                };
                // `record.message.clone()` is a `Bytes` clone: a refcount bump, not a copy of the
                // text, so a status read more than once (this function's contract allows it) does
                // not re-copy the message every time.
                // SAFETY: all three out-parameters checked non-null above.
                unsafe {
                    *out_code = record.code;
                    *out_msg = ak_bytes::from_bytes(record.message.clone());
                    *out_trailers = trailers;
                }
                crate::status::OK
            }
        }
    })
}

/// Ask the call to stop as soon as possible. Safe to call more than once, and safe to call on a
/// call that has already completed (a no-op in that case).
///
/// # Safety
///
/// `call` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn ak_call_cancel(call: *const ak_call) -> i32 {
    crate::guard::catch_unwind_status_only(|| {
        if call.is_null() {
            return crate::status::NULL_ARGUMENT;
        }
        if !live().contains(call) {
            return crate::status::INVALID_HANDLE;
        }
        // SAFETY: checked live above.
        unsafe { &*call }.cancel.cancel();
        crate::status::OK
    })
}

/// Release a call handle, safe to call at any point in the call's life cycle. If the call is still
/// running, it is cancelled and its driving task is forcibly stopped rather than left to finish on
/// its own time.
///
/// # Safety
///
/// `call` must be a value returned by [`ak_call_start`] that has not already been freed, or null (a
/// no-op).
#[no_mangle]
pub unsafe extern "C" fn ak_call_free(call: *mut ak_call) {
    crate::guard::catch_unwind_void(|| {
        if call.is_null() {
            return;
        }
        if !live().remove(call) {
            return;
        }
        // SAFETY: `live().remove` just returned `true`, so this address was produced by
        // `ak_call_start` and has not been freed since.
        let call = unsafe { Box::from_raw(call) };
        call.cancel.cancel();
        call.task.abort();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn publisher() -> (Arc<Mutex<Option<StatusRecord>>>, StatusPublisher) {
        let slot = Arc::new(Mutex::new(None));
        let event = OwnedEvent::new().expect("create an event");
        (Arc::clone(&slot), StatusPublisher::new(slot, event))
    }

    fn published_code(slot: &Mutex<Option<StatusRecord>>) -> Option<i32> {
        slot.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|record| record.code)
    }

    #[test]
    fn a_published_outcome_is_what_the_caller_reads() {
        let (slot, publisher) = publisher();

        publisher.publish(StatusRecord::cancelled());
        drop(publisher);

        assert_eq!(published_code(&slot), Some(Code::Cancelled as i32));
    }

    #[test]
    fn a_panic_while_driving_a_call_becomes_a_status_the_caller_can_read() {
        // The property that matters: a bug in this crate must reach .NET as a failed call carrying the
        // diagnosis, not as a call that reports itself finished with nothing to say — and certainly not
        // as a panic unwinding into managed code. Both halves are asserted, because a status with a
        // useless message is only half the job: the panic's own text is what a bug report is written
        // from, and it has nowhere else to go once the task that produced it is gone.
        let (slot, publisher) = publisher();

        let body: std::pin::Pin<Box<dyn Future<Output = StatusRecord> + Send>> = Box::pin(async {
            crate::test_support::yield_once().await;
            panic!("`send_item` called without first calling `poll_reserve`");
        });
        let mut outcome = Box::pin(outcome_of(body));
        publisher.publish(crate::test_support::block_on(outcome.as_mut()));

        let published = slot.lock().unwrap_or_else(PoisonError::into_inner);
        let record = published
            .as_ref()
            .expect("a status must have been published");
        assert_eq!(record.code, crate::status::INTERNAL_PANIC);
        let message = String::from_utf8_lossy(&record.message);
        assert!(
            message.contains("poll_reserve"),
            "the panic's own words are the diagnosis: {message}"
        );
    }

    #[test]
    fn dropping_without_publishing_reports_a_failure_rather_than_nothing() {
        // The invariant `ak_call_try_recv`'s "completed" state depends on. Without it, a driving task
        // that ended without publishing — a panic, say — leaves a call that reports itself completed
        // while `ak_call_status` answers `INVALID_STATE` forever, with nothing left to wait for.
        let (slot, publisher) = publisher();

        drop(publisher);

        assert_eq!(published_code(&slot), Some(crate::status::INTERNAL_PANIC));
    }

    #[test]
    fn a_late_publish_never_overwrites_the_outcome_the_caller_already_has() {
        // `ak_call_status` may be read as soon as the status appears, so replacing it afterwards would
        // change an answer the caller could already have acted on.
        let (slot, publisher) = publisher();

        publisher.publish(StatusRecord::ok(MetadataMap::new()));
        publisher.publish(StatusRecord::deadline_exceeded());
        drop(publisher);

        assert_eq!(published_code(&slot), Some(Code::Ok as i32));
    }
}
