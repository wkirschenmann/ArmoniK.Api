//! One HTTP/2 request, driven by a task that owns both halves of it.
//!
//! Every entry point here posts a command and returns; the task on the other end of that channel is
//! the only thing that ever invokes the caller's event callback. That is what makes the three rules
//! of the contract hold without a lock anywhere: one emitter, one channel, one terminal event.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use armonik_transport::reexports::h2;
use armonik_transport::reexports::http;
use armonik_transport::reexports::http_body_util::channel::{Channel, Sender};
use armonik_transport::reexports::http_body_util::BodyExt;
use armonik_transport::reexports::hyper::body::Incoming;
use armonik_transport::reexports::hyper_util::client::legacy::ResponseFuture;
use bytes::Bytes;
use tokio::sync::mpsc;

use crate::client::{ak_client, RequestBody};
use crate::error::{ak_bytes, ak_bytes_in, describe, FfiError};
use crate::event::ak_event;
use crate::handle::Registry;
use crate::status::ak_status;

/// The queue the caller's chunks reach the connection through.
///
/// Capacity one on purpose. `hyper` polls the body for chunk N+1 only once chunk N has been admitted
/// under the HTTP/2 flow-control window, so one slot is what turns the peer's window into
/// back-pressure the caller feels, with at most two chunks of this crate's memory in flight.
type RequestChannel = Channel<Bytes, AbortError>;

/// The error a cancelled request aborts its body with.
///
/// Its whole job is to carry an `h2::Reason` in its cause chain: `hyper` looks for one there and
/// uses it as the RST_STREAM code, falling back to INTERNAL_ERROR when it finds none. A cancelled
/// call has to reset with CANCEL, which a server reads as "the client went away" rather than as "the
/// client broke".
#[derive(Debug)]
struct AbortError(h2::Error);

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

/// The abort a cancellation puts on the wire.
fn cancelled() -> AbortError {
    AbortError(h2::Error::from(h2::Reason::CANCEL))
}

/// The response body, as `hyper` hands it back.
type ResponseBody = Incoming;

/// A failure raised by the client before any response header arrived.
type LegacyError = armonik_transport::reexports::hyper_util::client::legacy::Error;

/// A failure raised when a chunk cannot be queued: the body is gone.
type SendError = armonik_transport::reexports::http_body_util::channel::SendError;

/// A failure raised on the response body, after the headers.
type HyperError = armonik_transport::reexports::hyper::Error;

/// The event callback. One per request, invoked only from this library's own tasks.
///
/// `payload` is **borrowed** for the duration of the call: copy what is needed before returning, and
/// never release it. `code` is `AK_OK` on every event but a failed completion.
///
/// The callback must not block, must not unwind, and must not re-enter this library for the request
/// it is reporting on.
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
// invoked from any thread.
unsafe impl Send for EventSink {}
// SAFETY: as above. `Sync` because the driving loop borrows the sink rather than owning it, and only
// ever from that one task: there is no sharing to make safe here, only a raw pointer the
// auto-traits will not look through.
unsafe impl Sync for EventSink {}

impl EventSink {
    fn emit(&self, event: ak_event, payload: &[u8], code: i32) {
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
        crate::guard::catch_unwind_void(|| callback(self.ctx, event.kind(), view, code));
    }
}

/// What the caller asks the driving task to do next.
enum Command {
    /// Arm one write. The bytes are owned: the write event fires when the chunk is admitted, which
    /// is long after the entry point returned, so the caller's own buffer cannot be the one sent.
    Write(Bytes),
    /// End the request body cleanly.
    CloseSend,
    /// Arm one read.
    ArmRead,
    /// Reset the stream and finish.
    Cancel,
}

/// What the caller has done with the request body.
///
/// One lock rather than two flags, because arming a write and ending the body are decisions about
/// the same thing. Taken separately they interleave: a write that reads an open body and a close
/// that reads no armed write can both succeed, and the close then drops the sender before that
/// write is admitted, leaving it armed for an event that can never come.
#[derive(Default)]
struct SendState {
    /// Whether a write is armed and not yet resolved.
    armed: bool,
    /// Whether the body has been ended.
    closed: bool,
}

/// A request handle.
///
/// The task owns everything that matters; this is the caller's end of the command channel plus the
/// flags that say what is currently armed.
pub struct ak_request {
    commands: mpsc::UnboundedSender<Command>,
    read_armed: Arc<AtomicBool>,
    send: Arc<Mutex<SendState>>,
}

/// The live requests, by the address the caller holds.
fn live() -> &'static Registry<ak_request> {
    static LIVE: OnceLock<Registry<ak_request>> = OnceLock::new();
    LIVE.get_or_init(Registry::new)
}

/// A counted reference to a live request, or the status to return.
///
/// Counted, not borrowed: the caller may release the request from another thread at any moment, and
/// the entry point holding this has to finish reading either way.
fn borrow(request: *const ak_request) -> Result<Arc<ak_request>, i32> {
    if request.is_null() {
        return Err(ak_status::AK_NULL_ARGUMENT.code());
    }
    live()
        .get(request)
        .ok_or(ak_status::AK_INVALID_HANDLE.code())
}

/// Open a request on `client` and start driving it.
///
/// `headers_blob` is a key/value blob. Two pseudo-keys are required and are not sent as headers:
///
/// - `:method` - the HTTP method, for instance `POST`.
/// - `:url` - the absolute request URL, scheme and authority included. The connection pool keys on
///   it, so a path-only form is refused.
///
/// Every other key is sent as a request header, in the order given, duplicates included. A `-bin`
/// value passes through untouched: base64 is the caller's convention, not this library's.
///
/// On `AK_OK` a completion event is guaranteed to follow, exactly once, and `ctx` may be given up
/// when it arrives. On any other status no event ever arrives.
///
/// # Safety
///
/// `client` must be a live handle from [`crate::ak_client_create`]. `headers_blob` must point to
/// `len` readable bytes for the duration of the call. `on_event` must remain callable, and `ctx`
/// valid, until the completion event; releasing the handle does not end that, because events go on
/// being delivered until the completion whatever the caller does with its reference. `out` must be
/// a writable `ak_request*`, and receives a handle to be given up by exactly one
/// [`ak_request_release`]. `out_err`, when non-null, must be a writable [`ak_bytes`] and receives a
/// message to give up with [`crate::ak_bytes_release`].
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
        // SAFETY: documented as writable by this function's contract. Cleared first, so a caller
        // that only checks the handle cannot mistake an uninitialised slot for a request.
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
        let send = Arc::new(Mutex::new(SendState::default()));
        let (commands, command_rx) = mpsc::unbounded_channel();

        let task = Task {
            client,
            sink: EventSink { on_event, ctx },
            read_armed: Arc::clone(&read_armed),
            send: Arc::clone(&send),
        };
        crate::runtime::handle().spawn(task.drive(request, body_sender, command_rx));

        let handle = live().insert(ak_request {
            commands,
            read_armed,
            send,
        });
        // SAFETY: checked non-null above.
        unsafe { *out = handle.cast_mut() };
        ak_status::AK_OK.code()
    })
}

/// Arm one write of `len` bytes.
///
/// The bytes are copied before this returns, so the caller's buffer is free immediately. The write
/// event says the chunk was accepted by the connection, and is what permits the next write: arming a
/// second while one is outstanding, or writing after [`ak_request_close_send`], is refused with
/// `AK_INVALID_STATE`.
///
/// # Safety
///
/// `request` must be a live handle from [`ak_request_start`], and `data` readable for `len` bytes
/// for the duration of the call.
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
        {
            let mut send = request.send.lock().unwrap_or_else(PoisonError::into_inner);
            if send.closed || send.armed {
                return ak_status::AK_INVALID_STATE.code();
            }
            send.armed = true;
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
/// Refused with `AK_INVALID_STATE` while a write is still armed, and if the body has already been
/// ended.
///
/// # Safety
///
/// `request` must be a live handle from [`ak_request_start`].
#[no_mangle]
pub unsafe extern "C" fn ak_request_close_send(request: *const ak_request) -> i32 {
    crate::guard::catch_unwind_status_only(|| {
        let request = match borrow(request) {
            Ok(request) => request,
            Err(status) => return status,
        };
        {
            let mut send = request.send.lock().unwrap_or_else(PoisonError::into_inner);
            if send.armed || send.closed {
                return ak_status::AK_INVALID_STATE.code();
            }
            send.closed = true;
        }
        post(&request, Command::CloseSend)
    })
}

/// Arm one read.
///
/// Exactly one event follows: a read event with a chunk of the response body, or the completion.
/// Arming a second read while one is outstanding is refused with `AK_INVALID_STATE`.
///
/// # Safety
///
/// `request` must be a live handle from [`ak_request_start`].
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
            return ak_status::AK_INVALID_STATE.code();
        }
        post(&request, Command::ArmRead)
    })
}

/// Cancel the request.
///
/// Resets the stream with CANCEL and finishes; a completion event carrying `AK_CANCELLED` follows
/// unless the request had already completed, in which case this does nothing. Calling it more than
/// once is harmless.
///
/// # Safety
///
/// `request` must be a live handle from [`ak_request_start`].
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
        ak_status::AK_OK.code()
    })
}

/// Give up the caller's reference to a request, and cancel it.
///
/// Releasing before the completion is how a caller abandons a request it no longer wants. The
/// request is cancelled, and the completion still arrives: there is no second ownership rule for
/// that path. Whatever was rooted for `ctx` is given back at the completion, on every path, and this
/// is not that moment.
///
/// Never an abort of the task: the task is what resets the stream and lets the pool have its
/// connection back, so stopping it where it stands is how a peer is left waiting on a call nobody
/// is on the other end of - and how a caller is left waiting for a completion that never comes.
///
/// Null is accepted and does nothing.
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
        let _ = request.commands.send(Command::Cancel);
    });
}

/// Post a command, treating a closed channel as "the request is already over".
fn post(request: &ak_request, command: Command) -> i32 {
    match request.commands.send(command) {
        Ok(()) => ak_status::AK_OK.code(),
        Err(_) => ak_status::AK_INVALID_STATE.code(),
    }
}

/// Assemble the outgoing request from the caller's blob.
fn build_request(
    pairs: &crate::blob::Pairs<'_>,
    body: RequestChannel,
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

    // No `content-length`: the length of a streamed body is not known when its headers go out, which
    // is both true and what gRPC expects.
    let mut request = http::Request::new(erase(body));
    *request.method_mut() = method;
    *request.uri_mut() = url;
    *request.version_mut() = http::Version::HTTP_2;
    *request.headers_mut() = headers;
    Ok(request)
}

/// Box the body into the one type the pool was built for.
///
/// A pool fixes one body type for every request on it, and it is built long before any request
/// exists, so the queue behind a particular request has to lose its own type on the way in.
fn erase(body: RequestChannel) -> RequestBody {
    body.map_err(Into::into).boxed()
}

/// Reset the stream and let go of both halves of the request.
///
/// Both halves, because a stream has two. Aborting the body rather than dropping it is what chooses
/// the RST_STREAM code: `hyper` reads the reason out of the error's cause chain, and a caller that
/// gave up has to say CANCEL rather than have the body end as though it had finished sending.
/// Dropping the response is what stops `hyper` waiting for the rest of it. Before the headers there
/// is no response body to drop, and dropping the request future is what stops the attempt.
///
/// Shared by the two ways of giving up - the caller says so, or the time runs out - because the peer
/// cannot tell them apart and should not have to: either way nobody is listening any more.
fn give_up<R>(
    sender: &mut Option<Sender<Bytes, AbortError>>,
    pending_write: &mut Option<Bytes>,
    response: &mut Option<R>,
    body: &mut Option<ResponseBody>,
) {
    if let Some(sender) = sender.take() {
        sender.abort(cancelled());
    }
    drop(pending_write.take());
    drop(response.take());
    drop(body.take());
}

/// How a request ended. Rendered into the one completion event by [`Task::drive`].
enum Outcome {
    /// The response ended cleanly. The blob holds the trailers, and has no pairs in it when the
    /// stream ended without any.
    Ended(Vec<u8>),
    /// The request failed, with the status to report and the message to report it with.
    Failed(i32, String),
}

/// Everything the driving task needs that outlives a single command.
struct Task {
    /// Held for as long as the request runs, which is what lets a request outlive the
    /// `ak_client_release` that gave up the caller's own reference to the pool.
    client: Arc<ak_client>,
    sink: EventSink,
    read_armed: Arc<AtomicBool>,
    send: Arc<Mutex<SendState>>,
}

/// What one turn of the driving loop resolved.
///
/// The loop selects into this rather than handling each branch inside the `select!`: the handlers
/// mutate the very state the branch futures borrow, so they have to run after the select expression
/// has ended, not inside it.
enum Step {
    Command(Option<Command>),
    /// The configured `Timeout` elapsed.
    Deadline,
    Response(Result<http::Response<ResponseBody>, LegacyError>),
    Sent(Result<(), SendError>),
    Frame(Option<Result<http_body::Frame<Bytes>, HyperError>>),
}

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
            client,
            sink,
            read_armed,
            send,
        } = self;

        let inner: std::pin::Pin<Box<dyn std::future::Future<Output = Outcome> + Send>> =
            Box::pin(run(
                &client,
                client.timeout,
                &sink,
                &read_armed,
                &send,
                request,
                body_sender,
                commands,
            ));
        let outcome = crate::guard::catch_unwind_future(inner).await;

        // The single point where the completion is emitted. Every path above arrives here, a panic
        // inside the loop included, which is what makes "exactly once, and last" hold by
        // construction rather than by discipline at each `return`.
        match outcome {
            Ok(Outcome::Ended(trailers)) => {
                sink.emit(
                    ak_event::AK_EVENT_COMPLETED,
                    &trailers,
                    ak_status::AK_OK.code(),
                );
            }
            Ok(Outcome::Failed(code, message)) => {
                sink.emit(ak_event::AK_EVENT_COMPLETED, message.as_bytes(), code);
            }
            Err(panic) => {
                sink.emit(
                    ak_event::AK_EVENT_COMPLETED,
                    panic.as_bytes(),
                    ak_status::AK_INTERNAL_PANIC.code(),
                );
            }
        }
    }
}

/// The driving loop: commands in, events out, one terminal outcome.
#[allow(clippy::too_many_arguments)]
async fn run(
    client: &ak_client,
    timeout: Option<Duration>,
    sink: &EventSink,
    read_armed_flag: &AtomicBool,
    send_state: &Mutex<SendState>,
    request: http::Request<RequestBody>,
    body_sender: Sender<Bytes, AbortError>,
    mut commands: mpsc::UnboundedReceiver<Command>,
) -> Outcome {
    // Boxed so that `&mut` is a future regardless of whether `ResponseFuture` is `Unpin`, which is
    // not something this crate should have to depend on.
    let mut response: Option<std::pin::Pin<Box<ResponseFuture>>> =
        Some(Box::pin(client.pool.request(request)));
    let mut body: Option<ResponseBody> = None;
    let mut sender: Option<Sender<Bytes, AbortError>> = Some(body_sender);
    // The chunk of an armed write, kept until the send resolves. The send is restarted from it on
    // every turn of the loop the write survives, which sends nothing twice: a queue slot is reserved
    // and only filled once the future resolves, so a future dropped mid-way has queued nothing.
    let mut pending_write: Option<Bytes> = None;
    let mut read_armed = false;
    let mut commands_open = true;
    // `Timeout` bounds the whole life of a request, the wait for its headers included, because
    // nothing below the sender of a request has a notion of one taking too long. Kept as an instant
    // rather than a sleep held across the loop: the branch below rebuilds the sleep each turn, and
    // sleeping to a fixed instant is the same wait however often it is restarted.
    let deadline = timeout.map(|limit| tokio::time::Instant::now() + limit);
    // Whether the response has to be driven to its end with nothing armed.
    //
    // Set when the caller lets go of the command channel: no read will ever be armed again, and a
    // completion is owed on that path as on every other. A loop that polled the response only while
    // a read was armed would have no branch left enabled at all - which is not a park but a panic,
    // since a `select!` with every branch disabled has nothing to return.
    //
    // A frame read this way with nothing armed is discarded: the caller asked for no bytes, and
    // handing it some would break the rule that nothing arrives unarmed. What is wanted from the
    // response here is only how it ended.
    let mut drain_response = false;

    loop {
        let step = tokio::select! {
            // Commands first: a cancel that arrives at the same moment as a frame has to win, or the
            // caller sees an event for a request it has already given up on.
            biased;

            command = commands.recv(), if commands_open => Step::Command(command),

            () = async {
                tokio::time::sleep_until(deadline.expect("guarded by the precondition")).await
            }, if deadline.is_some() => Step::Deadline,

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
                if (read_armed || drain_response) && body.is_some() => Step::Frame(frame),
        };

        match step {
            // The command channel is gone, so nothing more can ever be asked of the request and
            // no read will ever be armed. The response is driven to its end from here because a
            // completion is owed on this path as on every other, and this loop would otherwise have
            // nothing left that could produce one.
            Step::Command(None) => {
                commands_open = false;
                drain_response = true;
            }

            Step::Command(Some(Command::Write(chunk))) => pending_write = Some(chunk),

            Step::Command(Some(Command::CloseSend)) => {
                // Dropping the sender is what ends the body: `hyper` then sends an empty END_STREAM
                // frame.
                drop(sender.take());
            }

            Step::Command(Some(Command::ArmRead)) => read_armed = true,

            Step::Command(Some(Command::Cancel)) => {
                give_up(&mut sender, &mut pending_write, &mut response, &mut body);
                return Outcome::Failed(
                    ak_status::AK_CANCELLED.code(),
                    String::from("the request was cancelled"),
                );
            }

            // The same teardown as a cancel, and said in one place rather than two: a request that
            // ran out of time and one the caller gave up on leave the stream in the same state, and
            // the peer has no way to tell them apart. Bounding the loop from outside instead would
            // put a second teardown next to this one, reached by dropping rather than by aborting,
            // and the request body would end as though the caller had finished sending.
            Step::Deadline => {
                give_up(&mut sender, &mut pending_write, &mut response, &mut body);
                return Outcome::Failed(
                    ak_status::AK_TIMEOUT.code(),
                    format!(
                        "the request did not complete within the configured Timeout ({limit:?})",
                        limit = timeout.expect("a deadline only exists when the option does")
                    ),
                );
            }

            Step::Response(Ok(reply)) => {
                let (parts, incoming) = reply.into_parts();
                let blob = match encode_response_headers(&parts) {
                    Ok(blob) => blob,
                    Err(error) => {
                        return Outcome::Failed(ak_status::AK_INTERNAL.code(), error.to_string())
                    }
                };
                response = None;
                body = Some(incoming);
                sink.emit(
                    ak_event::AK_EVENT_RESPONSE_HEADERS,
                    &blob,
                    ak_status::AK_OK.code(),
                );
            }

            // Before any header arrived: a connection that could not be made, or one that died
            // during the handshake. Nothing was answered, so nothing is retryable at this level.
            Step::Response(Err(error)) => {
                return Outcome::Failed(
                    ak_status::AK_CONNECTION_FAILED.code(),
                    describe_transport(&error),
                )
            }

            // Emitted here rather than from the body's `poll_frame`, which runs on `hyper`'s own
            // tasks: this loop owns the sender, and one emitter is what makes delivery serialised
            // per request without a lock.
            Step::Sent(Ok(())) => {
                pending_write = None;
                // Cleared before the event, and the lock given up before it too: the caller is
                // allowed to arm the next write from inside the callback, and would otherwise wait
                // on a lock this task is still holding.
                send_state
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .armed = false;
                sink.emit(ak_event::AK_EVENT_WRITE_DONE, &[], ak_status::AK_OK.code());
            }

            // The body was dropped on `hyper`'s side, which only happens once the request is over.
            // No write event: the completion already on its way resolves the armed write, which is
            // the general form of "nothing arrives unarmed".
            Step::Sent(Err(_)) => {
                pending_write = None;
                drop(sender.take());
            }

            Step::Frame(Some(Ok(frame))) => match frame.into_data() {
                Ok(chunk) => {
                    // Delivered only against an armed read. A frame taken while nothing is armed was
                    // read to find how the response ended, and the caller, having asked for no
                    // bytes, is owed none of it.
                    if !read_armed {
                        continue;
                    }
                    read_armed = false;
                    // Cleared before the event, never after: the caller is allowed to arm the next
                    // read from inside the callback.
                    read_armed_flag.store(false, Ordering::Release);
                    sink.emit(
                        ak_event::AK_EVENT_READ_DONE,
                        &chunk,
                        ak_status::AK_OK.code(),
                    );
                }
                // Trailers end the response: `hyper` delivers data frames, then at most one trailers
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
                        Err(error) => {
                            Outcome::Failed(ak_status::AK_INTERNAL.code(), error.to_string())
                        }
                    };
                }
            },

            Step::Frame(Some(Err(error))) => {
                return Outcome::Failed(ak_status::AK_TRANSPORT.code(), describe_transport(&error))
            }

            // A clean end of stream with no trailers. A trailers-only response looks like this too,
            // its `grpc-status` having arrived in the headers, and so does an early
            // RST_STREAM(NO_ERROR), which `hyper` already reports as an end rather than a failure.
            Step::Frame(None) => {
                return match crate::blob::encode_vec(std::iter::empty()) {
                    Ok(blob) => Outcome::Ended(blob),
                    Err(error) => Outcome::Failed(ak_status::AK_INTERNAL.code(), error.to_string()),
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
/// The reason is the part a reader acts on - REFUSED_STREAM is worth another attempt,
/// ENHANCE_YOUR_CALM is not - and it sits in a cause that the flattened message would otherwise
/// render as a bare "stream error".
fn describe_transport(error: &(dyn std::error::Error + 'static)) -> String {
    let mut message = describe(error);
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(step) = current {
        if let Some(reason) = step
            .downcast_ref::<h2::Error>()
            .and_then(armonik_transport::reexports::h2::Error::reason)
        {
            message.push_str(&format!(" (HTTP/2 {reason:?})"));
            break;
        }
        current = step.source();
    }
    message
}
