//! One HTTP/2 request, driven by a task that owns both halves of it.
//!
//! Every entry point here posts a command and returns; the task on the other end of that channel is
//! the only thing that ever invokes the caller's event callback. That is what makes the three rules
//! in the crate documentation hold without a lock anywhere: one emitter, one channel, one terminal
//! event.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use armonik_transport::reexports::h2;
use armonik_transport::reexports::http;
use armonik_transport::reexports::http_body_util;
use armonik_transport::reexports::hyper_util::client::legacy::ResponseFuture;
use bytes::Bytes;
use http_body_util::channel::{Channel, Sender};
use http_body_util::BodyExt;
use tokio::sync::mpsc;

use crate::client::{ak_client, Pool, ResponseBody};
use crate::error::{ak_bytes, ak_bytes_in, describe, FfiError};
use crate::handle::Registry;

/// The request body: a one-frame queue with an abort half.
///
/// Capacity one on purpose. hyper polls the body for chunk N+1 only once chunk N has been admitted
/// under the HTTP/2 flow-control window, so one slot is what turns the peer's window into
/// back-pressure the caller feels, with at most two chunks of this crate's memory in flight.
pub(crate) type RequestBody = Channel<Bytes, AbortError>;

/// The error a cancelled request aborts its body with.
///
/// Its whole job is to carry an `h2::Reason` in its cause chain: hyper looks for one there and uses
/// it as the RST_STREAM code, falling back to INTERNAL_ERROR when it finds none. A cancelled gRPC
/// call has to reset with CANCEL, which the server reads as "the client went away", not as "the
/// client broke".
#[derive(Debug)]
pub(crate) struct AbortError(h2::Error);

impl std::fmt::Display for AbortError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the request was cancelled")
    }
}

impl std::error::Error for AbortError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

fn cancelled() -> AbortError {
    AbortError(h2::Error::from(h2::Reason::CANCEL))
}

/// The event callback. One per request, invoked only from this crate's runtime threads.
///
/// `payload` is **borrowed** for the duration of the call: copy what is needed before returning, and
/// never free it. `code` is [`crate::status::OK`] on every event but a failed `COMPLETED`.
///
/// The callback must not block, must not unwind, and must not call back into this crate's request
/// functions for the same request from inside itself.
pub type ak_request_on_event =
    Option<extern "C" fn(ctx: *mut std::ffi::c_void, kind: i32, payload: ak_bytes_in, code: i32)>;

/// Where the driving task sends its events.
///
/// The raw `ctx` is what makes this not automatically `Send`; it is a token this crate never
/// dereferences, only hands back, so moving it to the task that owns the request is sound.
struct EventSink {
    on_event: ak_request_on_event,
    ctx: *mut std::ffi::c_void,
}

// SAFETY: `ctx` is opaque to this crate - never read, never written, only passed back to the
// callback the caller supplied alongside it. The caller's contract is that the callback may be
// invoked from any thread. `Sync` as well as `Send` because the driving task borrows the sink
// rather than owning it, and only ever from that one task: there is no sharing to make safe here,
// only a raw pointer the auto-traits will not look through.
unsafe impl Send for EventSink {}
// SAFETY: as above.
unsafe impl Sync for EventSink {}

impl EventSink {
    fn emit(&self, kind: i32, payload: &[u8], code: i32) {
        let Some(callback) = self.on_event else {
            return;
        };
        let view = ak_bytes_in {
            ptr: if payload.is_empty() {
                std::ptr::null()
            } else {
                payload.as_ptr()
            },
            len: payload.len(),
        };
        // A callback that unwinds is a contract violation, not something to let cross back into the
        // task and lose the request's terminal event over.
        crate::guard::catch_unwind_void(|| callback(self.ctx, kind, view, code));
    }
}

/// What the caller asks the driving task to do next.
enum Command {
    /// Arm one write. The bytes are owned: `WRITE_DONE` fires when the chunk is queued, which is
    /// before hyper has read it, so the caller's buffer cannot be the one that is sent.
    Write(Bytes),
    /// End the request body cleanly.
    CloseSend,
    /// Arm one read.
    ArmRead,
    /// Reset the stream and finish.
    Cancel,
}

/// A request handle.
///
/// The task owns everything that matters; this is the caller's end of the command channel plus the
/// flags that say what is currently armed.
pub struct ak_request {
    commands: mpsc::UnboundedSender<Command>,
    read_armed: Arc<AtomicBool>,
    write_armed: Arc<AtomicBool>,
    send_closed: AtomicBool,
}

fn live() -> &'static Registry<ak_request> {
    static LIVE: OnceLock<Registry<ak_request>> = OnceLock::new();
    LIVE.get_or_init(Registry::new)
}

/// A counted reference to a live request, or the status to return.
///
/// Counted, not borrowed: the caller may free the request from another thread at any moment, and
/// the entry point holding this has to finish reading either way.
fn borrow(request: *const ak_request) -> Result<Arc<ak_request>, i32> {
    if request.is_null() {
        return Err(crate::status::NULL_ARGUMENT);
    }
    live().get(request).ok_or(crate::status::INVALID_HANDLE)
}

/// Open a request and start driving it.
///
/// `headers_blob` is a key/value blob (see the header preamble). Two pseudo-keys are required and
/// are not sent as headers:
///
/// - `:method` - the HTTP method, e.g. `POST`.
/// - `:url` - the absolute request URL, scheme and authority included. The connection pool keys on
///   it, so a path-only form is refused.
///
/// Every other key is sent as a request header, in the order given, duplicates included. `-bin`
/// values are passed through untouched: base64 is the caller's convention, not this crate's.
///
/// On [`crate::status::OK`] a `COMPLETED` event is guaranteed to follow, exactly once, and that is
/// the moment - the only moment - at which `ctx` may be released. On any other status no event will
/// ever be delivered, and `ctx` may be released at once.
///
/// # Safety
///
/// `client` must be a live handle from [`crate::ak_client_create`]. `headers_blob` must point to
/// `len` readable bytes for the duration of the call. `on_event` must remain callable, and `ctx`
/// valid, until the `COMPLETED` event - [`ak_request_release`] does not end that obligation, it only
/// gives up the handle. `out` must be writable.
#[no_mangle]
pub unsafe extern "C" fn ak_request_start(
    client: *const ak_client,
    headers_blob: *const u8,
    len: usize,
    on_event: ak_request_on_event,
    ctx: *mut std::ffi::c_void,
    out: *mut *mut ak_request,
    out_err: *mut ak_bytes,
) -> i32 {
    crate::guard::catch_unwind_status(out_err, || {
        if out.is_null() {
            return FfiError::NullArgument("out").into_ffi_result(out_err);
        }
        // SAFETY: documented as writable.
        unsafe { *out = std::ptr::null_mut() };
        if client.is_null() {
            return FfiError::NullArgument("client").into_ffi_result(out_err);
        }
        if on_event.is_none() {
            return FfiError::NullArgument("on_event").into_ffi_result(out_err);
        }
        let Some(client) = crate::client::get(client) else {
            return FfiError::InvalidHandle.into_ffi_result(out_err);
        };

        // SAFETY: forwarded from this function's contract.
        let pairs = match unsafe { crate::blob::decode(headers_blob, len) } {
            Ok(pairs) => pairs,
            Err(error) => return error.into_ffi_result(out_err),
        };

        let (body_sender, body) = Channel::<Bytes, AbortError>::new(1);
        let request = match build_request(&pairs, body, client.user_agent.as_ref()) {
            Ok(request) => request,
            Err(error) => return error.into_ffi_result(out_err),
        };

        let read_armed = Arc::new(AtomicBool::new(false));
        let write_armed = Arc::new(AtomicBool::new(false));
        let (commands, command_rx) = mpsc::unbounded_channel();

        let task = Task {
            pool: Arc::clone(&client.pool),
            timeout: client.timeout,
            sink: EventSink { on_event, ctx },
            read_armed: Arc::clone(&read_armed),
            write_armed: Arc::clone(&write_armed),
        };
        crate::runtime::handle().spawn(task.drive(request, body_sender, command_rx));

        let handle = live().insert(ak_request {
            commands,
            read_armed,
            write_armed,
            send_closed: AtomicBool::new(false),
        });
        // SAFETY: checked non-null above.
        unsafe { *out = handle.cast_mut() };
        crate::status::OK
    })
}

/// Arm one write of `len` bytes.
///
/// The bytes are copied before this returns, so the caller's buffer is free immediately; the
/// `WRITE_DONE` event says the chunk was accepted by the connection, and is what permits the next
/// write. Arming a second write while one is outstanding, or writing after
/// [`ak_request_close_send`], returns [`crate::status::INVALID_STATE`].
///
/// # Safety
///
/// `request` must be a live handle, and `data` readable for `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn ak_request_write(
    request: *const ak_request,
    data: *const u8,
    len: usize,
) -> i32 {
    crate::guard::catch_unwind_status_only(|| {
        let request = match borrow(request) {
            Ok(request) => request,
            Err(status) => return status,
        };
        if request.send_closed.load(Ordering::Acquire) {
            return crate::status::INVALID_STATE;
        }
        if request
            .write_armed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return crate::status::INVALID_STATE;
        }

        let chunk = if data.is_null() || len == 0 {
            Bytes::new()
        } else {
            // SAFETY: forwarded from this function's contract.
            Bytes::copy_from_slice(unsafe { std::slice::from_raw_parts(data, len) })
        };
        post(&request, Command::Write(chunk))
    })
}

/// End the request body.
///
/// # Safety
///
/// `request` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn ak_request_close_send(request: *const ak_request) -> i32 {
    crate::guard::catch_unwind_status_only(|| {
        let request = match borrow(request) {
            Ok(request) => request,
            Err(status) => return status,
        };
        if request.write_armed.load(Ordering::Acquire) {
            return crate::status::INVALID_STATE;
        }
        if request.send_closed.swap(true, Ordering::AcqRel) {
            return crate::status::INVALID_STATE;
        }
        post(&request, Command::CloseSend)
    })
}

/// Arm one read.
///
/// Exactly one event follows: `READ_DONE` with a chunk of the response body, or `COMPLETED`.
///
/// # Safety
///
/// `request` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn ak_request_read(request: *const ak_request) -> i32 {
    crate::guard::catch_unwind_status_only(|| {
        let request = match borrow(request) {
            Ok(request) => request,
            Err(status) => return status,
        };
        if request
            .read_armed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return crate::status::INVALID_STATE;
        }
        post(&request, Command::ArmRead)
    })
}

/// Cancel the request.
///
/// Resets the stream with CANCEL and finishes; a `COMPLETED` event with
/// [`crate::status::CANCELLED`] follows unless the request had already completed, in which case this
/// does nothing. Calling it more than once is harmless.
///
/// # Safety
///
/// `request` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn ak_request_cancel(request: *const ak_request) -> i32 {
    crate::guard::catch_unwind_status_only(|| {
        let request = match borrow(request) {
            Ok(request) => request,
            Err(status) => return status,
        };
        // A closed channel means the task has already finished: the request is over, which is what
        // the caller wanted.
        let _ = request.commands.send(Command::Cancel);
        crate::status::OK
    })
}

/// Give up this caller's reference to a request.
///
/// A reference, not the object: the request is reference-counted, and whatever is still using it -
/// another thread mid-call, the task driving it - keeps it alive. Nothing is deallocated until the
/// last reference goes, so releasing a handle another thread is calling into is well-defined rather
/// than a race.
///
/// Releasing before `COMPLETED` is allowed and is how a request is abandoned: it cancels, and the
/// request runs to its `COMPLETED` event as it always does. **That event still arrives**, and it
/// remains the only point at which `ctx` may be released - this call does not end that obligation.
/// A caller that must know no callback can reach it any more, an AppDomain being unloaded for
/// instance, waits for the outstanding `COMPLETED` events rather than for this to return.
///
/// Calling it twice, or on a handle that was never valid, does nothing.
///
/// # Safety
///
/// `request` must be a handle from [`ak_request_start`] that has not been released, or null.
#[no_mangle]
pub unsafe extern "C" fn ak_request_release(request: *mut ak_request) {
    crate::guard::catch_unwind_void(|| {
        if request.is_null() {
            return;
        }
        let Some(request) = live().remove(request) else {
            return;
        };
        // Cancelled, so the task does not linger on a request nobody is going to read. It still
        // runs to `COMPLETED`; that is what tells the caller its context may go.
        let _ = request.commands.send(Command::Cancel);
    });
}

/// Post a command, treating a closed channel as "the request is already over".
fn post(request: &ak_request, command: Command) -> i32 {
    match request.commands.send(command) {
        Ok(()) => crate::status::OK,
        Err(_) => crate::status::INVALID_STATE,
    }
}

/// Assemble the outgoing request from the caller's blob.
fn build_request(
    pairs: &crate::blob::Pairs<'_>,
    body: RequestBody,
    user_agent: Option<&http::HeaderValue>,
) -> Result<http::Request<RequestBody>, FfiError> {
    let mut method: Option<http::Method> = None;
    let mut url: Option<http::Uri> = None;
    let mut headers = http::HeaderMap::new();

    for (key, value) in pairs {
        match *key {
            b":method" => {
                method = Some(http::Method::from_bytes(value).map_err(|error| {
                    FfiError::InvalidRequest(format!("`:method` is not a method: {error}"))
                })?);
            }
            b":url" => {
                url = Some(http::Uri::try_from(*value).map_err(|error| {
                    FfiError::InvalidRequest(format!("`:url` is not a URI: {error}"))
                })?);
            }
            other if other.starts_with(b":") => {
                return Err(FfiError::InvalidRequest(format!(
                    "unknown pseudo-header `{}`",
                    String::from_utf8_lossy(other)
                )));
            }
            name => {
                let name = http::HeaderName::from_bytes(name).map_err(|error| {
                    FfiError::InvalidRequest(format!("not a header name: {error}"))
                })?;
                let value = http::HeaderValue::from_bytes(value).map_err(|error| {
                    FfiError::InvalidRequest(format!("not a header value for `{name}`: {error}"))
                })?;
                headers.append(name, value);
            }
        }
    }

    let method = method.ok_or(FfiError::InvalidRequest(String::from(
        "`:method` is missing",
    )))?;
    let url = url.ok_or(FfiError::InvalidRequest(String::from("`:url` is missing")))?;
    if url.scheme().is_none() || url.authority().is_none() {
        return Err(FfiError::InvalidRequest(format!(
            "`:url` must be absolute, with a scheme and an authority: `{url}`"
        )));
    }

    if let Some(agent) = user_agent {
        if !headers.contains_key(http::header::USER_AGENT) {
            headers.append(http::header::USER_AGENT, agent.clone());
        }
    }

    // No `content-length`: the body's size is unknown, which is both true and what gRPC expects.
    let mut request = http::Request::new(body);
    *request.method_mut() = method;
    *request.uri_mut() = url;
    *request.version_mut() = http::Version::HTTP_2;
    *request.headers_mut() = headers;
    Ok(request)
}

/// How a request ended. Rendered into the one `COMPLETED` event by [`Task::drive`].
enum Outcome {
    /// The response ended cleanly. The blob holds the trailers, and is empty of pairs when the
    /// stream ended without any - which is also what an early RST_STREAM(NO_ERROR) looks like,
    /// since hyper already reports that as a clean end rather than a failure.
    Ended(Vec<u8>),
    Failed(i32, String),
}

/// Everything the driving task needs that outlives a single command.
struct Task {
    pool: Arc<Pool>,
    timeout: Option<Duration>,
    sink: EventSink,
    read_armed: Arc<AtomicBool>,
    write_armed: Arc<AtomicBool>,
}

/// What one turn of the driving loop resolved.
///
/// The loop selects into this rather than handling each branch inside the `select!`: the handlers
/// mutate the very state the branch futures borrow, so they have to run after the select expression
/// has ended, not inside it.
enum Step {
    Command(Option<Command>),
    Response(Result<http::Response<ResponseBody>, LegacyError>),
    Sent(Result<(), http_body_util::channel::SendError>),
    Frame(Option<Result<http_body::Frame<Bytes>, HyperError>>),
}

type LegacyError = armonik_transport::reexports::hyper_util::client::legacy::Error;
type HyperError = armonik_transport::reexports::hyper::Error;
use armonik_transport::reexports::hyper::body as http_body;

impl Task {
    /// Run the request to its single terminal event.
    async fn drive(
        self,
        request: http::Request<RequestBody>,
        body_sender: Sender<Bytes, AbortError>,
        commands: mpsc::UnboundedReceiver<Command>,
    ) {
        let Task {
            pool,
            timeout,
            sink,
            read_armed,
            write_armed,
        } = self;

        let inner: std::pin::Pin<Box<dyn std::future::Future<Output = Outcome> + Send>> =
            Box::pin(run(
                pool,
                &sink,
                &read_armed,
                &write_armed,
                request,
                body_sender,
                commands,
            ));
        let caught = crate::guard::catch_unwind_future(inner);

        // Nothing at the `hyper_util` level implements a whole-request deadline, so it is applied
        // here, around the loop and everything it holds. gRPC deadlines do not come through this:
        // they arrive as a cancellation from the caller's own timer.
        let outcome = match timeout {
            Some(limit) => match tokio::time::timeout(limit, caught).await {
                Ok(caught) => caught,
                Err(_) => Ok(Outcome::Failed(
                    crate::status::TIMEOUT,
                    format!(
                        "the request did not complete within the configured Timeout ({limit:?})"
                    ),
                )),
            },
            None => caught.await,
        };

        // The single point where `COMPLETED` is emitted. Every path above arrives here, a panic
        // inside the loop included, which is what makes rule 2 hold by construction rather than by
        // discipline at each `return`.
        match outcome {
            Ok(Outcome::Ended(trailers)) => {
                sink.emit(crate::event::COMPLETED, &trailers, crate::status::OK);
            }
            Ok(Outcome::Failed(code, message)) => {
                sink.emit(crate::event::COMPLETED, message.as_bytes(), code);
            }
            Err(panic) => {
                sink.emit(
                    crate::event::COMPLETED,
                    panic.as_bytes(),
                    crate::status::INTERNAL_PANIC,
                );
            }
        }
    }
}

/// The driving loop: commands in, events out, one terminal outcome.
#[allow(clippy::too_many_arguments)]
async fn run(
    pool: Arc<Pool>,
    sink: &EventSink,
    read_armed_flag: &AtomicBool,
    write_armed_flag: &AtomicBool,
    request: http::Request<RequestBody>,
    body_sender: Sender<Bytes, AbortError>,
    mut commands: mpsc::UnboundedReceiver<Command>,
) -> Outcome {
    // Boxed so `&mut` is a future regardless of whether `ResponseFuture` is `Unpin`, which is not
    // something this crate should have to depend on.
    let mut response: Option<std::pin::Pin<Box<ResponseFuture>>> =
        Some(Box::pin(pool.request(request)));
    let mut body: Option<ResponseBody> = None;
    let mut sender: Option<Sender<Bytes, AbortError>> = Some(body_sender);
    // The chunk of an armed write. Re-sent from scratch on every turn of the loop it survives:
    // `Sender::send_data` either resolves having queued the chunk or is dropped having queued
    // nothing, so restarting it is not a way to send anything twice.
    let mut pending_write: Option<Bytes> = None;
    let mut read_armed = false;
    let mut commands_open = true;

    loop {
        let step = tokio::select! {
            // Commands first: a Cancel that arrives at the same moment as a frame has to win, or
            // the caller sees an event for a request it has already given up on.
            biased;

            command = commands.recv(), if commands_open => Step::Command(command),

            result = async { response.as_mut().expect("guarded by the precondition").await },
                if response.is_some() => Step::Response(result),

            result = async {
                let chunk = pending_write.clone().expect("guarded by the precondition");
                sender
                    .as_mut()
                    .expect("guarded by the precondition")
                    .send_data(chunk)
                    .await
            }, if pending_write.is_some() && sender.is_some() => Step::Sent(result),

            frame = async { body.as_mut().expect("guarded by the precondition").frame().await },
                if read_armed && body.is_some() => Step::Frame(frame),
        };

        match step {
            // The handle was dropped without this crate being told; nothing more will ever be
            // asked of the request, but the response still has to run to its end.
            Step::Command(None) => commands_open = false,

            Step::Command(Some(Command::Write(chunk))) => pending_write = Some(chunk),

            Step::Command(Some(Command::CloseSend)) => {
                // Dropping the sender is what ends the body: hyper then sends an empty END_STREAM
                // frame. Aborting it would reset the stream instead.
                sender = None;
            }

            Step::Command(Some(Command::ArmRead)) => read_armed = true,

            Step::Command(Some(Command::Cancel)) => {
                // Both halves have to go. Aborting the body is what puts CANCEL on the wire, and
                // dropping the response is what stops hyper from waiting for the rest of it; either
                // one alone leaves the stream open as far as the peer is concerned.
                if let Some(sender) = sender.take() {
                    sender.abort(cancelled());
                }
                drop(pending_write.take());
                drop(response.take());
                drop(body.take());
                return Outcome::Failed(
                    crate::status::CANCELLED,
                    String::from("the request was cancelled"),
                );
            }

            Step::Response(Ok(reply)) => {
                let (parts, incoming) = reply.into_parts();
                let blob = match encode_response_headers(&parts) {
                    Ok(blob) => blob,
                    Err(error) => {
                        return Outcome::Failed(crate::status::INTERNAL, error.to_string())
                    }
                };
                response = None;
                body = Some(incoming);
                sink.emit(crate::event::RESPONSE_HEADERS, &blob, crate::status::OK);
            }

            // Before any header arrived: a connection that could not be made, or one that died
            // during the handshake. Nothing was answered, so nothing is retryable at this level.
            Step::Response(Err(error)) => {
                return Outcome::Failed(
                    crate::status::CONNECTION_FAILED,
                    describe_transport(&error),
                )
            }

            Step::Sent(Ok(())) => {
                pending_write = None;
                // Cleared before the event, never after: the caller is allowed to arm the next
                // write from inside the callback.
                write_armed_flag.store(false, Ordering::Release);
                sink.emit(crate::event::WRITE_DONE, &[], crate::status::OK);
            }

            // The body was dropped on hyper's side, which only happens once the request is over.
            // No `WRITE_DONE`: the `COMPLETED` that is already on its way resolves the armed write,
            // which is the general form of rule 1.
            Step::Sent(Err(_)) => {
                pending_write = None;
                sender = None;
            }

            Step::Frame(Some(Ok(frame))) => match frame.into_data() {
                Ok(chunk) => {
                    read_armed = false;
                    read_armed_flag.store(false, Ordering::Release);
                    sink.emit(crate::event::READ_DONE, &chunk, crate::status::OK);
                }
                // Trailers end the response: hyper delivers data frames, then at most one trailers
                // frame, then nothing.
                Err(frame) => {
                    let trailers = frame
                        .into_trailers()
                        .expect("a frame is either data or trailers");
                    return match crate::blob::encode_vec(
                        trailers
                            .iter()
                            .map(|(name, value)| (name.as_str().as_bytes(), value.as_bytes())),
                    ) {
                        Ok(blob) => Outcome::Ended(blob),
                        Err(error) => Outcome::Failed(crate::status::INTERNAL, error.to_string()),
                    };
                }
            },

            Step::Frame(Some(Err(error))) => {
                return Outcome::Failed(crate::status::TRANSPORT, describe_transport(&error))
            }

            // A clean end of stream with no trailers. A trailers-only response looks like this too,
            // and so does an early RST_STREAM(NO_ERROR), which hyper already reports as an end
            // rather than as a failure.
            Step::Frame(None) => {
                return match crate::blob::encode_vec(std::iter::empty()) {
                    Ok(blob) => Outcome::Ended(blob),
                    Err(error) => Outcome::Failed(crate::status::INTERNAL, error.to_string()),
                }
            }
        }
    }
}

/// Encode the response status and headers as one blob.
fn encode_response_headers(parts: &http::response::Parts) -> Result<Vec<u8>, FfiError> {
    let status = parts.status.as_u16().to_string();
    let pairs = std::iter::once((&b":status"[..], status.as_bytes())).chain(
        parts
            .headers
            .iter()
            .map(|(name, value)| (name.as_str().as_bytes(), value.as_bytes())),
    );
    crate::blob::encode_vec(pairs)
}

/// Render a transport failure, appending the HTTP/2 reason when there is one.
///
/// The reason is the part a reader acts on - REFUSED_STREAM is retryable, ENHANCE_YOUR_CALM is not -
/// and it is buried in a cause the flattened message would otherwise render as a bare "stream
/// error".
fn describe_transport(error: &(dyn std::error::Error + 'static)) -> String {
    let mut message = describe(error);
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(step) = current {
        if let Some(h2) = step.downcast_ref::<h2::Error>() {
            if let Some(reason) = h2.reason() {
                message.push_str(&format!(" (HTTP/2 {reason:?})"));
                break;
            }
        }
        current = step.source();
    }
    message
}
