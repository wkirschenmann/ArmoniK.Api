#![allow(non_camel_case_types)]

// A point a test hooks to make the code panic there: nothing without the test hooks. Defined before
// the modules, which is what makes it visible in them.
macro_rules! at {
    ($hook:ident, $($step:tt)+) => {
        #[cfg(feature = "test-hooks")]
        crate::hooks::$hook(crate::hooks::$($step)+);
    };
}

mod abi;
mod blob;
mod call;
mod channel;
mod config;
mod held;
#[cfg(feature = "test-hooks")]
pub mod hooks;
mod host;
mod ledger;
mod lifecycle;
mod log;
mod refusal;
mod registry;
mod runtime;
mod spares;
mod tables;
mod tagged;

use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

pub use abi::*;
use host::{Host, HostPtr};
use refusal::Refusal;
use registry::Registry;

fn guard_with<T>(fallback: T, body: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(body)).unwrap_or(fallback)
}

/// Runs an entry point's body, a panic answering AK_STATUS_INTERNAL.
fn guard(body: impl FnOnce() -> Result<(), Refusal>) -> Result<(), Refusal> {
    catch_unwind(AssertUnwindSafe(body)).unwrap_or(Err(PANICKED))
}

const PANICKED: Refusal = Refusal::fixed(
    ak_status::AK_STATUS_INTERNAL,
    ak_error_kind::AK_ERROR_NONE,
    "this library panicked inside the downcall",
);

/// A status as an answer: OK is success, anything else the refusal the status alone makes.
fn done(status: ak_status) -> Result<(), Refusal> {
    match status {
        ak_status::AK_STATUS_OK => Ok(()),
        refused => Err(refused.into()),
    }
}

fn guard_void(body: impl FnOnce()) {
    let _ = catch_unwind(AssertUnwindSafe(body));
}

/// A task body whose panic answers `None` instead of ending the task in silence.
///
/// The entry-point guards above cannot serve here: a future panics inside a `poll`, so the catch
/// has to be per poll rather than around the whole thing. `Box::pin` because that makes the
/// projection safe code.
///
/// What a caller owes on `None` is whatever the task promised someone else - a call's terminal, a
/// shutdown's completion - because a task that dies quietly leaves a host waiting for an event
/// that will never come. Requirement 14.8 is that promise: contained, and converted to an error.
async fn guarded<T>(body: impl std::future::Future<Output = T>) -> Option<T> {
    let mut body = Box::pin(body);
    std::future::poll_fn(move |cx| {
        match catch_unwind(AssertUnwindSafe(|| body.as_mut().poll(cx))) {
            Ok(std::task::Poll::Pending) => std::task::Poll::Pending,
            Ok(std::task::Poll::Ready(value)) => std::task::Poll::Ready(Some(value)),
            Err(_) => std::task::Poll::Ready(None),
        }
    })
    .await
}

/// # Safety
///
/// `out` must be writable for its type.
unsafe fn hand_over<T, R: Into<Refusal>>(out: *mut T, made: Result<T, R>) -> Result<(), Refusal> {
    let value = made.map_err(Into::into)?;
    unsafe { *out = value };
    Ok(())
}

/// Looks a handle up and writes what it answers, leaving `*out` untouched on any refusal.
///
/// The shape every entry point with an out parameter keeps: a null out is INVALID_ARG, a handle
/// this table does not hold is HANDLE_STALE, and the reader's own refusal is whatever it says.
/// `ak_call_start` is the one that does not use it - it reads its options before the lookup, so
/// folding it would change which status wins when the options are null and the channel is stale
/// too, and the two are different codes to the host.
///
/// # Safety
///
/// `out` must be null or writable for its type.
unsafe fn observe<T, V, R: Into<Refusal>>(
    table: &Registry<T>,
    handle: ak_handle,
    out: *mut V,
    read: impl FnOnce(&Arc<T>) -> Result<V, R>,
) -> Result<(), Refusal> {
    if out.is_null() {
        return Err(ak_status::AK_STATUS_INVALID_ARG.into());
    }
    let found = table.get(handle).ok_or(ak_status::AK_STATUS_HANDLE_STALE)?;
    unsafe { hand_over(out, read(&found)) }
}

/// Creates a runtime, synchronously; it starts in AK_RUNTIME_RUNNING. One exists at a time: a
/// second create before the first is destroyed is refused with AK_STATUS_INVALID_STATE. Each
/// channel created on it runs on a thread of its own, so that a call never moves between threads.
/// AK_RUNTIME_QUIESCENT means those threads are gone too.
///
/// # Safety
///
/// `config` and `out` must be valid for their types, and `callback` must stay callable with
/// `runtime_ctx` until the runtime's last event. `config.log_callback`, when it is set, must stay
/// callable with `config.log_ctx` until `ak_runtime_destroy` returns, or until this call returns
/// when it refuses.
/// `out_error` must be null or writable for an `ak_error`.
#[no_mangle]
pub unsafe extern "C" fn ak_runtime_create(
    config: *const ak_runtime_config,
    callback: ak_callback,
    runtime_ctx: *mut c_void,
    out: *mut ak_handle,
    out_error: *mut ak_error,
) -> ak_status {
    let answered = guard(|| {
        let (Some(callback), false, false) = (callback, config.is_null(), out.is_null()) else {
            return Err(NULL_ARGUMENT);
        };
        let config = unsafe { read_versioned(config) }?;
        let defaults = unsafe { config.channel_defaults_json.as_slice() }.ok_or(NULL_SLICE)?;

        unsafe {
            hand_over(
                out,
                lifecycle::create_runtime(
                    config.memory_ceiling,
                    config.memory_hard_ceiling,
                    defaults,
                    log::Sink::new(config.log_callback, config.log_ctx),
                    Host::new(callback, runtime_ctx),
                ),
            )
        }
    });
    unsafe { refusal::answer(out_error, answered) }
}

const NULL_ARGUMENT: Refusal = Refusal::fixed(
    ak_status::AK_STATUS_INVALID_ARG,
    ak_error_kind::AK_ERROR_USAGE,
    "a pointer argument is null",
);

/// Creates a runtime, as ak_runtime_create does, from the sources `config` lists: read in order,
/// a later one over an earlier one option by option, into the vocabulary of runtime.schema.json -
/// the endpoint, the memory ceilings, and the channel defaults every channel's own document is
/// merged over.
///
/// What is malformed in `config` itself - a kind it does not name, a reserved field or a flag it
/// does not know, a value on an environment source, a byte view that is null with a length or not
/// UTF-8 - is AK_STATUS_INVALID_ARG before any source is read. A source
/// that is refused is AK_STATUS_INVALID_ARG too, its message naming the source and the key's path,
/// never the value; so is a loaded option the runtime cannot be created with: a ceiling of zero,
/// an Endpoint that is not a URI, or channel defaults a channel's own document would be refused
/// for.
///
/// # Safety
///
/// `config` and `out` must be valid for their types, `config.sources` must point at
/// `source_count` sources unless that is zero, and every byte view at its length. `callback` must
/// stay callable with `runtime_ctx` until the runtime's last event. `config.log_callback`, when it
/// is set, must stay callable with `config.log_ctx` until `ak_runtime_destroy` returns, or until
/// this call returns when it refuses.
/// `out_error` must be null or writable for an `ak_error`.
#[no_mangle]
pub unsafe extern "C" fn ak_runtime_create_from(
    config: *const ak_config,
    callback: ak_callback,
    runtime_ctx: *mut c_void,
    out: *mut ak_handle,
    out_error: *mut ak_error,
) -> ak_status {
    let answered = guard(|| {
        let (Some(callback), false, false) = (callback, config.is_null(), out.is_null()) else {
            return Err(NULL_ARGUMENT);
        };
        let config = unsafe { read_versioned(config) }?;
        let configuration = unsafe { config::sources(&config) }?;

        unsafe {
            hand_over(
                out,
                lifecycle::create_runtime_from(
                    &configuration,
                    log::Sink::new(config.log_callback, config.log_ctx),
                    Host::new(callback, runtime_ctx),
                ),
            )
        }
    });
    unsafe { refusal::answer(out_error, answered) }
}

/// The runtime's state. Synchronous, non-blocking, and callable from any thread, including from
/// inside a callback.
#[no_mangle]
pub extern "C" fn ak_runtime_status(runtime: ak_handle) -> ak_runtime_state {
    guard_with(
        ak_runtime_state::AK_RUNTIME_FAILED_UNQUIESCED,
        || match tables::runtimes().get(runtime) {
            Some(runtime) => runtime.state(),
            None => ak_runtime_state::AK_RUNTIME_NONE,
        },
    )
}

/// Closes the start gate and drains. AK_EVENT_SHUTDOWN_COMPLETE follows. Idempotent.
///
/// # Safety
///
/// `out_error` must be null or writable for an `ak_error`.
#[no_mangle]
pub unsafe extern "C" fn ak_runtime_begin_shutdown(
    runtime: ak_handle,
    out_error: *mut ak_error,
) -> ak_status {
    let answered = guard(|| {
        let runtime = tables::runtimes()
            .get(runtime)
            .ok_or(ak_status::AK_STATUS_HANDLE_STALE)?;
        lifecycle::begin_shutdown(&runtime);
        Ok(())
    });
    unsafe { refusal::answer(out_error, answered) }
}

/// Frees the runtime. Refused before AK_RUNTIME_QUIESCENT, and that is the only reason.
///
/// Live call and channel handles do not block it: destroy stales every handle of this runtime at
/// once, and a later downcall on one returns AK_STATUS_HANDLE_STALE. A handle names runtime-owned
/// state, so the runtime may reclaim it; a payload or a lent buffer is memory the host may still
/// be reading or writing, so only the host can end it.
///
/// # Safety
///
/// `out_error` must be null or writable for an `ak_error`.
#[no_mangle]
pub unsafe extern "C" fn ak_runtime_destroy(
    runtime: ak_handle,
    out_error: *mut ak_error,
) -> ak_status {
    let answered = guard(|| done(lifecycle::destroy_runtime(runtime)));
    unsafe { refusal::answer(out_error, answered) }
}

/// What the runtime-wide byte ceiling is holding. Synchronous, non-blocking and observational: it
/// changes nothing.
///
/// # Safety
///
/// `out` must be writable.
/// `out_error` must be null or writable for an `ak_error`.
#[no_mangle]
pub unsafe extern "C" fn ak_runtime_memory_usage(
    runtime: ak_handle,
    out: *mut ak_memory_usage,
    out_error: *mut ak_error,
) -> ak_status {
    let answered = guard(|| unsafe {
        observe(tables::runtimes(), runtime, out, |found| {
            Ok::<_, ak_status>(found.ledger().usage())
        })
    });
    unsafe { refusal::answer(out_error, answered) }
}

/// Creates a channel on an endpoint, configured by a JSON document. Synchronous: it reads the
/// certificate files the document names, and resolves no name and opens no socket until the
/// channel's first call. A bad endpoint or a bad document is AK_STATUS_INVALID_ARG, a file that
/// cannot be read or holds nothing usable included, and so is a null out or a null slice with a
/// non-zero length; a runtime handle that names nothing is AK_STATUS_HANDLE_STALE, and one that is
/// shutting down is AK_STATUS_INVALID_STATE.
///
/// The endpoint is its own argument, as UTF-8 - "http://host:port" in the clear, or
/// "https://host:port" over TLS. An empty one is the Endpoint of the runtime's configuration, and
/// is AK_STATUS_INVALID_ARG when that names none. Every option of the document has a default, and
/// `{}` is a valid configuration.
///
/// The document is structured and typed, and a JSON schema states it: objects nest, and a number is
/// a number and not a string spelled like one. A key no option declares is refused by its path,
/// at the root of the document as below it. That schema, `options.schema.json`, names each option
/// with its type and, where it has them, its range and default.
///
/// ak_channel_delivery_window reads back the delivery window the channel ended up with.
///
/// The two windows mirror each other. Grpc.Host.Receive.Window bounds the payloads of one call
/// outstanding at once, each taking one credit, one place of the window - the terminal status takes
/// none, so a host holds at most one more - and the host chooses it because the host is what has
/// to hold them. Grpc.Host.Send.Window
/// bounds the buffers one call may have out, counting those being filled and those awaiting their
/// WRITE_DONE. Grpc.Host.Receive.Window defaults to 4 and Grpc.Host.Send.Window to 1, and
/// the schema states the range either may take.
///
/// A call whose payloads the host does not consume reads a few messages past its spent credits,
/// which the engine holds outside what the credits count, and then stops reading its stream; what
/// its peer sends meanwhile holds the connection's HTTP/2 window. Every call of the channel shares
/// that window, so enough unread calls stop the others receiving.
///
/// # Safety
///
/// `config_json` must point at its bytes for the duration of the call, and `out` must be
/// writable.
/// `out_error` must be null or writable for an `ak_error`.
#[no_mangle]
pub unsafe extern "C" fn ak_channel_create(
    runtime: ak_handle,
    endpoint: ak_bytes_in,
    config_json: ak_bytes_in,
    out: *mut ak_handle,
    out_error: *mut ak_error,
) -> ak_status {
    let answered = guard(|| {
        observe(tables::runtimes(), runtime, out, |found| {
            // Held across the create, so a shutdown that closed the gate cannot miss the
            // channel this is about to insert.
            let _pass = found.pass_the_gate().ok_or(RUNTIME_STOPPING)?;
            let endpoint = unsafe { endpoint.as_slice() }.ok_or(NULL_SLICE)?;
            let json = unsafe { config_json.as_slice() }.ok_or(NULL_SLICE)?;
            channel::create(runtime, found, endpoint, json)
        })
    });
    unsafe { refusal::answer(out_error, answered) }
}

const RUNTIME_STOPPING: Refusal = Refusal::fixed(
    ak_status::AK_STATUS_INVALID_STATE,
    ak_error_kind::AK_ERROR_USAGE,
    "the runtime is shutting down and takes no new channel or call",
);
const NULL_SLICE: Refusal = Refusal::fixed(
    ak_status::AK_STATUS_INVALID_ARG,
    ak_error_kind::AK_ERROR_USAGE,
    "a byte view has a null pointer and a length that is not zero",
);

/// Frees the channel, cancelling its calls first.
///
/// The cancellation is not a courtesy: the channel is closing from this moment and no closing
/// channel may have an active call, so the latch is what makes the drain this library's business
/// rather than something the host must provoke. A call parked on a delivery credit is the case
/// that needs it - it is not watching the transport, so closing the session alone would never
/// reach it.
///
/// Idempotent. The channel goes to AK_CHANNEL_CLOSING, and once its last call has reached its
/// terminal the handle is reclaimed: from then on ak_channel_status answers AK_CHANNEL_NONE and
/// ak_call_start AK_STATUS_HANDLE_STALE. Until then the channel starts no further call -
/// ak_call_start on it answers AK_STATUS_INVALID_STATE - and a host following the drain reads
/// CLOSING and then NONE, with CLOSED at most in passing. An idle channel is reclaimed before this
/// returns.
///
/// A channel the runtime's shutdown closed is not reclaimed: it stays AK_CHANNEL_CLOSED, and
/// nameable, until this is called or the runtime is destroyed.
#[no_mangle]
pub extern "C" fn ak_channel_release(channel: ak_handle) {
    guard_void(|| lifecycle::release_channel(channel));
}

/// How far along a channel's closing is. Answers NONE for a handle this library does not know,
/// which a released channel is once its last call has ended.
///
/// What ends CLOSING is this library's own bookkeeping - the last call of the channel reaching
/// its terminal - so a host watching the drain has nothing to do but read. In particular CLOSED
/// does not wait for the host to consume what it has been given: a call past its terminal still
/// owes its payloads, and the channel is closed regardless.
#[no_mangle]
pub extern "C" fn ak_channel_status(channel: ak_handle) -> ak_channel_state {
    guard_with(
        ak_channel_state::AK_CHANNEL_NONE,
        || match tables::channels().get(channel) {
            Some(found) => found.state(),
            None => ak_channel_state::AK_CHANNEL_NONE,
        },
    )
}

/// The delivery window a channel was created with: Grpc.Host.Receive.Window as the channel's own
/// document, the runtime's channel defaults or this library's default settled it. Fixed for the
/// life of the channel.
///
/// A handle this library does not know is AK_STATUS_HANDLE_STALE, and a null out
/// AK_STATUS_INVALID_ARG.
///
/// # Safety
///
/// `out` must be writable.
/// `out_error` must be null or writable for an `ak_error`.
#[no_mangle]
pub unsafe extern "C" fn ak_channel_delivery_window(
    channel: ak_handle,
    out: *mut u32,
    out_error: *mut ak_error,
) -> ak_status {
    let answered = guard(|| unsafe {
        observe(tables::channels(), channel, out, |found| {
            Ok::<_, ak_status>(u32::try_from(found.delivery_credits).unwrap_or(u32::MAX))
        })
    });
    unsafe { refusal::answer(out_error, answered) }
}

/// Starts a call on a channel.
///
/// There is no ak_call_release, on purpose. Every resource a call lends out is given back through
/// something the runtime already observes, so it knows when a terminal call owes nothing and takes
/// the handle back itself. Two consequences: the handle goes stale at a moment the host does not
/// choose, and abandoning a call is still ak_call_cancel followed by consuming through to the
/// terminal - dropping a payload on the floor keeps the runtime alive.
///
/// # Safety
///
/// `options` must be valid for its type and its byte views, and `out` must be writable.
/// `out_error` must be null or writable for an `ak_error`.
#[no_mangle]
pub unsafe extern "C" fn ak_call_start(
    channel: ak_handle,
    options: *const ak_call_start_options,
    call_ctx: ak_call_ctx,
    out: *mut ak_handle,
    out_error: *mut ak_error,
) -> ak_status {
    let answered = guard(|| {
        if options.is_null() || out.is_null() {
            return Err(NULL_ARGUMENT);
        }
        let options = unsafe { read_versioned(options) }?;

        let found = tables::channels()
            .get(channel)
            .ok_or(ak_status::AK_STATUS_HANDLE_STALE)?;
        let runtime = tables::runtimes()
            .get(found.runtime)
            .ok_or(ak_status::AK_STATUS_HANDLE_STALE)?;
        let _pass = runtime.pass_the_gate().ok_or(RUNTIME_STOPPING)?;
        if found.state() != ak_channel_state::AK_CHANNEL_OPEN {
            return Err(CHANNEL_CLOSING);
        }

        let method = unsafe { options.method.as_slice() }.ok_or(NULL_SLICE)?;
        let metadata = unsafe { options.metadata.as_slice() }.ok_or(NULL_SLICE)?;
        let method = std::str::from_utf8(method).map_err(|_| METHOD_NOT_UTF8)?;
        let metadata = blob::decode_metadata(metadata).ok_or(METADATA_UNREADABLE)?;
        let deadline = (options.flags & AK_CALL_HAS_DEADLINE != 0).then(|| {
            armonik_transport::grpc::Deadline::Timeout(std::time::Duration::from_nanos(
                options.timeout_ns,
            ))
        });
        let one_request = options.flags & AK_CALL_ONE_REQUEST != 0;
        let one_response = options.flags & AK_CALL_ONE_RESPONSE != 0;
        let wait_for_ready = options.flags & AK_CALL_WAIT_FOR_READY != 0;

        unsafe {
            hand_over(
                out,
                call::start_on(
                    &found,
                    &runtime.services(),
                    method,
                    metadata,
                    deadline,
                    call::Shape {
                        one_request,
                        one_response,
                        wait_for_ready,
                    },
                    HostPtr(call_ctx),
                ),
            )
        }
    });
    unsafe { refusal::answer(out_error, answered) }
}

const CHANNEL_CLOSING: Refusal = Refusal::fixed(
    ak_status::AK_STATUS_INVALID_STATE,
    ak_error_kind::AK_ERROR_USAGE,
    "the channel is closing and starts no new call",
);
const METHOD_NOT_UTF8: Refusal = Refusal::fixed(
    ak_status::AK_STATUS_INVALID_ARG,
    ak_error_kind::AK_ERROR_USAGE,
    "the method is not UTF-8",
);
const METADATA_UNREADABLE: Refusal = Refusal::fixed(
    ak_status::AK_STATUS_INVALID_ARG,
    ak_error_kind::AK_ERROR_USAGE,
    "the metadata is not a blob of the layout the header states",
);

/// Lends a buffer out of the call's arena to serialize into, of `len` bytes at most: the host
/// writes from its start and says how many bytes it wrote when it commits it. The buffer holds
/// whatever the allocator, or the last message the channel lent it for, left there, never read:
/// only the bytes the host says it wrote are sent.
/// Writing past `len` is an overrun, which the commit or the return may detect by the bytes this
/// library put after the end: a write that changes them is AK_STATUS_CORRUPTED, and the runtime
/// shuts down. A write that leaves them as they were goes unseen.
///
/// One unfilled buffer at a time, whatever Grpc.Host.Send.Window says: asking for a second while
/// still holding one, or while an earlier lend of the call has not returned, is
/// AK_STATUS_INVALID_STATE, a host bug rather than backpressure. The window
/// counts those being filled and those committed and awaiting their WRITE_DONE; when it is full
/// the refusal is AK_STATUS_SLOT_BUSY, whose wake-up is this call's next WRITE_DONE. That wake-up
/// is only meaningful because a host eligible to ask holds nothing. AK_STATUS_BUDGET_BUSY is the
/// runtime-wide ceiling, whose wake-up is the call's next AK_EVENT_BUDGET_WAKE. A host retries
/// only after the refused lend has returned: until then a retry can meet AK_STATUS_INVALID_STATE.
/// AK_STATUS_MESSAGE_TOO_LARGE is permanent. A length of zero is AK_STATUS_INVALID_ARG: an empty
/// message needs no buffer, and ak_call_send_message sends one with none. An allocator failure for
/// the buffer is AK_STATUS_INTERNAL: that lend is refused, and nothing else fails. So is a panic the
/// library contains, which can only come before the host holds the buffer: the lend is refused as
/// any other, with nothing charged, no slot of the window spent, the call's one buffer free and no
/// wait for room recorded, so the host may ask again. A call that is over, or whose cancellation
/// has been requested, lends nothing: AK_STATUS_INVALID_STATE; nor does a call that declared
/// AK_CALL_ONE_REQUEST once its request is committed, no WRITE_DONE coming for a SLOT_BUSY to wait
/// on. On every refusal no buffer is lent and `*out` is untouched.
///
/// # Safety
///
/// `out` must be writable.
/// `out_error` must be null or writable for an `ak_error`.
#[no_mangle]
pub unsafe extern "C" fn ak_get_call_buffer(
    call: ak_handle,
    len: usize,
    out: *mut ak_buffer,
    out_error: *mut ak_error,
) -> ak_status {
    let answered = guard(|| {
        if len == 0 {
            return Err(EMPTY_LEND);
        }
        unsafe { observe(tables::calls(), call, out, |found| found.lend(len)) }
    });
    unsafe { refusal::answer(out_error, answered) }
}

const EMPTY_LEND: Refusal = Refusal::fixed(
    ak_status::AK_STATUS_INVALID_ARG,
    ak_error_kind::AK_ERROR_USAGE,
    "a lend of no bytes: an empty message is sent with no buffer",
);

/// Commits a lent buffer as the next message, the `written` bytes the host wrote from its start.
/// Ownership passes back to this library.
///
/// More than the buffer's length, or a write past its end that changed the bytes after it, is
/// AK_STATUS_CORRUPTED: the memory around the buffer may be corrupted, so the buffer is taken
/// back without being freed, nothing of it is sent, and the runtime shuts down. The buffer is then
/// this library's: giving it back again is a use of memory the host no longer owns.
///
/// An empty message takes no buffer: `buffer` is then the empty one, owner NULL and len 0, with
/// `written` 0, and the send takes a slot of the window as any other, AK_STATUS_SLOT_BUSY when
/// there is none. A zeroed `ak_buffer` is that empty one, so committing the zeroed `*out` of a
/// refused ak_get_call_buffer, which the refusal leaves untouched, sends an empty message.
///
/// AK_EVENT_WRITE_DONE settles an accepted send and frees its slot from the moment the event is
/// emitted, not when the callback returns - so a host woken by it may ask for a buffer from inside
/// the callback. It says nothing about the network: the message may have been written, or
/// abandoned because the call was cancelled. It arrives exactly once per accepted send, in send
/// order, and always before the terminal - but on a call that declared AK_CALL_ONE_REQUEST, whose
/// commit also ends the sending and settles the send: no WRITE_DONE comes for it.
///
/// Refused with AK_STATUS_INVALID_STATE once the call is over or its cancellation has been
/// requested, after ak_call_end_send, and after a one-request call's commit. The buffer then stays
/// the host's, to give back with ak_return_call_buffer.
///
/// A panic the library contains is answered by what the commit had done. Before the buffer is taken
/// to be the message it is AK_STATUS_INTERNAL, a refusal like the others: the buffer stays lent and
/// the host's, to commit again or give back. Once the message is queued it is AK_STATUS_OK. Between
/// the two the buffer is gone and the answer is AK_STATUS_CORRUPTED: it is taken back and the
/// runtime shuts down.
///
/// # Safety
///
/// `buffer` must be one this call lent and the host has not given back, and the host must have
/// written its first `written` bytes: they are sent as they are.
/// `out_error` must be null or writable for an `ak_error`.
#[no_mangle]
pub unsafe extern "C" fn ak_call_send_message(
    call: ak_handle,
    buffer: ak_buffer,
    written: usize,
    out_error: *mut ak_error,
) -> ak_status {
    let answered = guard(|| {
        if buffer.owner.is_null() && buffer.len == 0 && written == 0 {
            let found = tables::calls()
                .get(call)
                .ok_or(ak_status::AK_STATUS_HANDLE_STALE)?;
            return done(found.commit_empty());
        }
        let mut lent = (unsafe { call::take_lent(buffer.owner) }).ok_or(NOT_LENT)?;
        at!(at_send_step, SendStep::Taken);
        // Before anything else: past an overrun, nothing the host passes alongside can be trusted.
        if lent.seal(written).is_err() {
            Arc::clone(lent.call()).overrun(lent.into_box());
            return Err(OVERRUN);
        }
        at!(at_send_step, SendStep::Sealed);
        let Some(found) = tables::calls().get(call) else {
            return done(lent.keep(ak_status::AK_STATUS_HANDLE_STALE));
        };
        // The buffer names its own call, so a handle that names another one is the host's
        // mistake and not this library's to resolve.
        if !Arc::ptr_eq(lent.call(), &found) {
            lent.keep(ak_status::AK_STATUS_INVALID_ARG);
            return Err(ANOTHER_CALLS_BUFFER);
        }
        at!(at_send_step, SendStep::Resolved);
        match found.commit(lent) {
            ak_status::AK_STATUS_CORRUPTED => Err(LOST_TO_A_PANIC),
            status => done(status),
        }
    });
    unsafe { refusal::answer(out_error, answered) }
}

const NOT_LENT: Refusal = Refusal::fixed(
    ak_status::AK_STATUS_INVALID_ARG,
    ak_error_kind::AK_ERROR_USAGE,
    "the buffer is not one this library lent",
);
const OVERRUN: Refusal = Refusal::fixed(
    ak_status::AK_STATUS_CORRUPTED,
    ak_error_kind::AK_ERROR_USAGE,
    "the buffer was written past its end, or committed longer than it was lent: memory may be \
     corrupted, and the runtime is shutting down",
);
const LOST_TO_A_PANIC: Refusal = Refusal::fixed(
    ak_status::AK_STATUS_CORRUPTED,
    ak_error_kind::AK_ERROR_NONE,
    "this library panicked after it took the buffer for the message: the buffer is gone, and the \
     runtime is shutting down",
);
const ANOTHER_CALLS_BUFFER: Refusal = Refusal::fixed(
    ak_status::AK_STATUS_INVALID_ARG,
    ak_error_kind::AK_ERROR_USAGE,
    "the buffer was lent to another call than the one the handle names",
);

/// Gives a lent buffer back unused. Legal on a cancelled or terminal call: it is the only exit for
/// a buffer whose send is refused, and the call is not reclaimed until it happens.
///
/// Takes no call handle: the buffer determines its call. A refused ak_call_send_message therefore
/// leaves the buffer with the host, except for AK_STATUS_CORRUPTED, which takes it back. A buffer
/// given back with the bytes after its end changed is an overrun, as at the commit: it is taken
/// back without being freed and the runtime shuts down, with no status to say so but the shutdown.
/// So is a buffer whose bytes after its end could not be read for a contained panic. Any other
/// panic leaves the return made: the buffer is given back and its debt paid.
///
/// # Safety
///
/// `buffer` must be one this call lent and the host has not given back.
#[no_mangle]
pub unsafe extern "C" fn ak_return_call_buffer(buffer: ak_buffer) {
    guard_void(|| {
        let Some(lent) = (unsafe { call::take_lent(buffer.owner) }) else {
            return;
        };
        Arc::clone(lent.call()).return_buffer(lent);
    });
}

/// Exchanges a lent buffer for one of `new_len` bytes, larger or smaller, carrying over the first
/// `keep` bytes the host wrote: a host that finds its buffer too small asks for another size
/// without giving the message up. It is a lend of the new length and a return of the old one, and
/// the ceiling sees one step: the charge moves by the difference, and a growth is admitted as a
/// lend of that difference would be. The ceiling counts the charge, not the instant: the old
/// arena is still allocated while the bytes are copied.
///
/// On success `*out` is the new buffer, whose first `keep` bytes are the old one's and whose
/// others are as a lend leaves them. The host gives back `*out`, in its turn, and never `buffer`:
/// the old memory may be reused at once. The host's one buffer and its slot of the send window
/// are the new buffer's.
///
/// On every refusal but AK_STATUS_CORRUPTED the old buffer is still lent and the host's, as it
/// was, and `*out` is untouched. AK_STATUS_BUDGET_BUSY says the ceiling has no room for the new
/// size now: it records no wait and owes no AK_EVENT_BUDGET_WAKE, and a host that waits for room
/// gives the buffer back and lends the new length, as one that waits holds none.
/// AK_STATUS_MESSAGE_TOO_LARGE is permanent. A call that is over, or whose cancellation has been
/// requested, resizes nothing, nor does one that declared AK_CALL_ONE_REQUEST and has committed
/// it: AK_STATUS_INVALID_STATE. A `new_len` of zero, a `keep` past
/// `new_len`, a null `out` and a `buffer` that is not lent are AK_STATUS_INVALID_ARG. An
/// allocator failure is AK_STATUS_INTERNAL, and so is a panic before the exchange is made, the
/// ceiling's charge included, which is made whole or not at all: it is a refusal like the others,
/// with the old buffer lent and charged, so the host may retry or give it back. A panic after it,
/// while the old memory is set aside, does not undo it: the answer is AK_STATUS_OK. A panic while
/// taking back an overrun is AK_STATUS_CORRUPTED.
///
/// A `keep` past the length the buffer was lent at, or a write past its end that changed the bytes
/// after it, is an overrun, as it is at the commit: AK_STATUS_CORRUPTED, the buffer taken back
/// without being freed, nothing carried over, and the runtime shutting down.
///
/// # Safety
///
/// `buffer` must be one this call lent and the host has not given back, and the host must have
/// written its first `keep` bytes: they are carried over as they are.
/// `out` must be writable.
/// `out_error` must be null or writable for an `ak_error`.
#[no_mangle]
pub unsafe extern "C" fn ak_resize_call_buffer(
    buffer: ak_buffer,
    new_len: usize,
    keep: usize,
    out: *mut ak_buffer,
    out_error: *mut ak_error,
) -> ak_status {
    let answered = guard(|| {
        if out.is_null() {
            return Err(ak_status::AK_STATUS_INVALID_ARG.into());
        }
        if new_len == 0 {
            return Err(EMPTY_LEND);
        }
        if keep > new_len {
            return Err(KEEP_PAST_THE_NEW_LENGTH);
        }
        let lent = (unsafe { call::take_lent(buffer.owner) }).ok_or(NOT_LENT)?;
        let call = Arc::clone(lent.call());
        match call.resize(lent, new_len, keep) {
            Ok(resized) => {
                unsafe { *out = resized };
                Ok(())
            }
            Err(ak_status::AK_STATUS_CORRUPTED) => Err(OVERRUN_ON_RESIZE),
            Err(refused) => Err(refused.into()),
        }
    });
    unsafe { refusal::answer(out_error, answered) }
}

const KEEP_PAST_THE_NEW_LENGTH: Refusal = Refusal::fixed(
    ak_status::AK_STATUS_INVALID_ARG,
    ak_error_kind::AK_ERROR_USAGE,
    "the bytes to keep are more than the new length holds",
);
const OVERRUN_ON_RESIZE: Refusal = Refusal::fixed(
    ak_status::AK_STATUS_CORRUPTED,
    ak_error_kind::AK_ERROR_USAGE,
    "the buffer was written past its end, or kept longer than it was lent: memory may be \
     corrupted, and the runtime is shutting down",
);

/// Signals end of sending. No ak_call_send_message after this: a send that comes after the end,
/// and a second end, answer AK_STATUS_INVALID_STATE. So does any end on a call that declared
/// AK_CALL_ONE_REQUEST, whose commit ends the sending, and which a host that wants no request
/// cancels.
///
/// An end that comes while an ak_call_send_message has not yet returned on another thread - from
/// inside that send's own AK_EVENT_WRITE_DONE, which may arrive first - waits for it to return, so
/// it never goes ahead of a message the host has been told left.
///
/// # Safety
///
/// `out_error` must be null or writable for an `ak_error`.
#[no_mangle]
pub unsafe extern "C" fn ak_call_end_send(call: ak_handle, out_error: *mut ak_error) -> ak_status {
    let answered = guard(|| {
        let found = tables::calls()
            .get(call)
            .ok_or(ak_status::AK_STATUS_HANDLE_STALE)?;
        done(found.end_send())
    });
    unsafe { refusal::answer(out_error, answered) }
}

/// Cancels the call, which then reaches a terminal - carrying CANCELLED, unless the peer's own
/// status was already in.
///
/// Asynchronous: the request takes effect when the call's task observes it, so callbacks already
/// committed may still arrive after this returns, and a status the peer had already sent is the
/// one delivered. INITIAL_METADATA is never skipped.
///
/// # Safety
///
/// `out_error` must be null or writable for an `ak_error`.
#[no_mangle]
pub unsafe extern "C" fn ak_call_cancel(call: ak_handle, out_error: *mut ak_error) -> ak_status {
    let answered = guard(|| {
        let found = tables::calls()
            .get(call)
            .ok_or(ak_status::AK_STATUS_HANDLE_STALE)?;
        found.cancel();
        Ok(())
    });
    unsafe { refusal::answer(out_error, answered) }
}

/// What the call still owes. Purely observational; it is legal never to call it. It exists because
/// without a release downcall nothing reports a forgotten ak_return_call_buffer synchronously.
///
/// # Safety
///
/// `out` must be writable.
/// `out_error` must be null or writable for an `ak_error`.
#[no_mangle]
pub unsafe extern "C" fn ak_call_debt_of(
    call: ak_handle,
    out: *mut ak_call_debt,
    out_error: *mut ak_error,
) -> ak_status {
    let answered = guard(|| unsafe {
        observe(tables::calls(), call, out, |found| {
            Ok::<_, ak_status>(found.debt())
        })
    });
    unsafe { refusal::answer(out_error, answered) }
}

#[no_mangle]
pub extern "C" fn ak_abi_version() -> i32 {
    AK_ABI_VERSION
}

/// Signals that the host has consumed an event's payload. Two things at once:
///
/// 1. frees the native memory;
/// 2. arms reception of the next event for that call.
///
/// At most Grpc.Host.Receive.Window non-consumed payloads per call, four by default: while the
/// host owes them, the runtime withholds the next data event. Only a terminal still goes out with
/// every credit - every place of the window - spent.
///
/// The host MUST give a call's payloads back in delivery order: with several outstanding, the
/// oldest is the next one consumed. Another order is not refused - this library frees whichever
/// owner it is given - but the guarantees this header states are proved for this order alone.
///
/// Remains legal, and required, after the terminal: the call is not reclaimed until it happens.
///
/// # Safety
///
/// `payload` must be one this library delivered and the host has not consumed.
#[no_mangle]
pub unsafe extern "C" fn ak_event_consumed(payload: ak_bytes) {
    guard_void(|| {
        drop(unsafe { call::take_payload(payload.owner) });
    });
}

/// Gives back several payloads in one downcall, in delivery order, each as ak_event_consumed gives
/// back one: an unowned payload, owner NULL, is a no-op, and a zero-length payload with an owner is
/// given back with its credit. `payloads` may be NULL when `count` is zero.
///
/// # Safety
///
/// `payloads` must point at `count` payloads this library delivered and the host has not
/// consumed.
#[no_mangle]
pub unsafe extern "C" fn ak_events_consumed(payloads: *const ak_bytes, count: usize) {
    guard_void(|| {
        if payloads.is_null() || count == 0 {
            return;
        }
        for payload in unsafe { std::slice::from_raw_parts(payloads, count) } {
            drop(unsafe { call::take_payload(payload.owner) });
        }
    });
}

/// Frees an ak_error's detail. A no-op when detail.owner is NULL, so a host may route every error
/// through it. Legal in any runtime state, and after ak_runtime_destroy.
///
/// # Safety
///
/// `detail` must be one this library wrote into an ak_error and the host has not released.
#[no_mangle]
pub unsafe extern "C" fn ak_error_release(detail: ak_bytes) {
    guard_void(|| unsafe { refusal::release(detail.owner) });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Silences the panic report the guarded tests provoke, so a passing run is quiet.
    fn quietly<T>(body: impl FnOnce() -> T) -> T {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let answer = body();
        std::panic::set_hook(previous);
        answer
    }

    #[test]
    fn a_task_body_that_finishes_answers_what_it_returned() {
        let answer = futures_lite_block_on(guarded(async { 7 }));
        assert_eq!(answer, Some(7));
    }

    #[test]
    fn a_task_body_that_panics_before_it_suspends_answers_none() {
        let answer = quietly(|| futures_lite_block_on(guarded(async { panic!("at once") })));
        assert_eq!(answer, None::<()>);
    }

    /// The reason this is not the synchronous `guard` beside it: a future panics inside a `poll`,
    /// and a catch around the whole future would be entered once and never again.
    #[test]
    fn a_task_body_that_panics_after_it_suspends_answers_none() {
        let answer = quietly(|| {
            futures_lite_block_on(guarded(async {
                Yielded { done: false }.await;
                panic!("after a suspension");
            }))
        });
        assert_eq!(answer, None::<()>);
    }

    /// One suspension, so the body above is polled twice.
    struct Yielded {
        done: bool,
    }

    impl std::future::Future for Yielded {
        type Output = ();

        fn poll(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<()> {
            if self.done {
                return std::task::Poll::Ready(());
            }
            self.done = true;
            cx.waker().wake_by_ref();
            std::task::Poll::Pending
        }
    }

    /// A one-thread executor, so these tests need no runtime of their own.
    fn futures_lite_block_on<T>(body: impl std::future::Future<Output = T>) -> T {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a current-thread runtime");
        runtime.block_on(body)
    }
}
