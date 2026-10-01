#![allow(non_camel_case_types)]

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
mod registry;
mod runtime;
mod tables;

use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

pub use abi::*;
use host::{Host, HostPtr};
use registry::Registry;

fn guard_with<T>(fallback: T, body: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(body)).unwrap_or(fallback)
}

fn guard(body: impl FnOnce() -> ak_status) -> ak_status {
    guard_with(ak_status::AK_STATUS_INTERNAL, body)
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
unsafe fn hand_over<T>(out: *mut T, made: Result<T, ak_status>) -> ak_status {
    match made {
        Err(status) => status,
        Ok(value) => {
            unsafe { *out = value };
            ak_status::AK_STATUS_OK
        }
    }
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
unsafe fn observe<T, V>(
    table: &Registry<T>,
    handle: ak_handle,
    out: *mut V,
    read: impl FnOnce(&Arc<T>) -> Result<V, ak_status>,
) -> ak_status {
    if out.is_null() {
        return ak_status::AK_STATUS_INVALID_ARG;
    }
    let Some(found) = table.get(handle) else {
        return ak_status::AK_STATUS_HANDLE_STALE;
    };
    match read(&found) {
        Err(status) => status,
        Ok(value) => {
            unsafe { *out = value };
            ak_status::AK_STATUS_OK
        }
    }
}

/// Creates a runtime. One exists at a time: a second create before the first is destroyed is
/// refused with AK_STATUS_INVALID_STATE.
///
/// # Safety
///
/// `config` and `out` must be valid for their types, and `callback` must stay callable with
/// `runtime_ctx` until the runtime's last event.
#[no_mangle]
pub unsafe extern "C" fn ak_runtime_create(
    config: *const ak_runtime_config,
    callback: ak_callback,
    runtime_ctx: *mut c_void,
    out: *mut ak_handle,
) -> ak_status {
    guard(|| {
        let (Some(callback), false, false) = (callback, config.is_null(), out.is_null()) else {
            return ak_status::AK_STATUS_INVALID_ARG;
        };
        let Some(config) = (unsafe { read_versioned(config) }) else {
            return ak_status::AK_STATUS_INVALID_ARG;
        };

        unsafe {
            hand_over(
                out,
                lifecycle::create_runtime(
                    config.worker_threads,
                    config.memory_ceiling,
                    Host::new(callback, runtime_ctx),
                ),
            )
        }
    })
}

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
#[no_mangle]
pub extern "C" fn ak_runtime_begin_shutdown(runtime: ak_handle) -> ak_status {
    guard(|| match tables::runtimes().get(runtime) {
        None => ak_status::AK_STATUS_HANDLE_STALE,
        Some(runtime) => {
            lifecycle::begin_shutdown(&runtime);
            ak_status::AK_STATUS_OK
        }
    })
}

/// Frees the runtime. Refused before AK_RUNTIME_QUIESCENT, and that is the only reason.
///
/// Live call and channel handles do not block it: destroy stales every handle of this runtime at
/// once, and a later downcall on one returns AK_STATUS_HANDLE_STALE. A handle names runtime-owned
/// state, so the runtime may reclaim it; a payload or a lent buffer is memory the host may still
/// be reading or writing, so only the host can end it.
#[no_mangle]
pub extern "C" fn ak_runtime_destroy(runtime: ak_handle) -> ak_status {
    guard(|| lifecycle::destroy_runtime(runtime))
}

/// # Safety
///
/// `out` must be writable.
#[no_mangle]
pub unsafe extern "C" fn ak_runtime_memory_usage(
    runtime: ak_handle,
    out: *mut ak_memory_usage,
) -> ak_status {
    guard(|| unsafe {
        observe(tables::runtimes(), runtime, out, |found| {
            Ok(found.ledger().usage())
        })
    })
}

/// Creates a channel on an endpoint, configured by a JSON document. Synchronous and performs no
/// I/O: no name is resolved and no socket opened until the channel's first call. A bad endpoint or
/// a bad document is AK_STATUS_INVALID_ARG, and so is a null out or a null slice with a non-zero
/// length; a runtime handle that names nothing is AK_STATUS_HANDLE_STALE, and one that is shutting
/// down is AK_STATUS_INVALID_STATE.
///
/// The endpoint is its own argument, as UTF-8 - "http://host:port". It is the one value a channel
/// cannot be created without, so it is not an option that happens to be mandatory: every option of
/// the document has a default, and `{}` is a valid configuration.
///
/// The document is structured and typed, and a JSON schema states it: objects nest, a number is a
/// number and not a string spelled like one, and an option spelled wrong is refused rather than
/// ignored. It carries UserAgent, MaxReceiveMessageSize, DeliveryCredits, MaxSendsInFlight, and
/// a Transport object holding ConnectTimeoutSeconds.
///
/// The two windows mirror each other. DeliveryCredits bounds the payloads of one call outstanding
/// at once - the terminal status takes no credit, so a host holds at most one more - and the host
/// chooses it because the host is what has to hold them; MaxSendsInFlight bounds the buffers one
/// call may have out, counting those being filled and those awaiting their WRITE_DONE. Both
/// default to 1, and the schema states the range either may take.
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
#[no_mangle]
pub unsafe extern "C" fn ak_channel_create(
    runtime: ak_handle,
    endpoint: ak_bytes_in,
    config_json: ak_bytes_in,
    out: *mut ak_handle,
) -> ak_status {
    guard(|| {
        observe(tables::runtimes(), runtime, out, |found| {
            // Held across the create, so a shutdown that closed the gate cannot miss the channel
            // this is about to insert.
            let _pass = found
                .pass_the_gate()
                .ok_or(ak_status::AK_STATUS_INVALID_STATE)?;
            let endpoint =
                unsafe { endpoint.as_slice() }.ok_or(ak_status::AK_STATUS_INVALID_ARG)?;
            let json = unsafe { config_json.as_slice() }.ok_or(ak_status::AK_STATUS_INVALID_ARG)?;
            channel::create(runtime, found.spawner(), endpoint, json)
        })
    })
}

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
#[no_mangle]
pub unsafe extern "C" fn ak_call_start(
    channel: ak_handle,
    options: *const ak_call_start_options,
    call_ctx: ak_call_ctx,
    out: *mut ak_handle,
) -> ak_status {
    guard(|| {
        if options.is_null() || out.is_null() {
            return ak_status::AK_STATUS_INVALID_ARG;
        }
        let Some(options) = (unsafe { read_versioned(options) }) else {
            return ak_status::AK_STATUS_INVALID_ARG;
        };

        let Some(found) = tables::channels().get(channel) else {
            return ak_status::AK_STATUS_HANDLE_STALE;
        };
        let Some(runtime) = tables::runtimes().get(found.runtime) else {
            return ak_status::AK_STATUS_HANDLE_STALE;
        };
        let Some(_pass) = runtime.pass_the_gate() else {
            return ak_status::AK_STATUS_INVALID_STATE;
        };
        if found.state() != ak_channel_state::AK_CHANNEL_OPEN {
            return ak_status::AK_STATUS_INVALID_STATE;
        }

        let (Some(method), Some(metadata)) = (unsafe { options.method.as_slice() }, unsafe {
            options.metadata.as_slice()
        }) else {
            return ak_status::AK_STATUS_INVALID_ARG;
        };
        let (Ok(method), Some(metadata)) =
            (std::str::from_utf8(method), blob::decode_metadata(metadata))
        else {
            return ak_status::AK_STATUS_INVALID_ARG;
        };

        unsafe {
            hand_over(
                out,
                call::start_on(
                    &found,
                    &runtime.services(),
                    method,
                    metadata,
                    HostPtr(call_ctx),
                ),
            )
        }
    })
}

/// Lends a buffer out of the call's arena to serialize into. The exact length is known before the
/// first byte is written, so no growable writer is needed.
///
/// One unfilled buffer at a time, whatever max_sends_in_flight says: asking for a second while
/// still holding one is AK_STATUS_INVALID_STATE, a host bug rather than backpressure. The window
/// counts those being filled and those committed and awaiting their WRITE_DONE; when it is full
/// the refusal is AK_STATUS_SLOT_BUSY, whose wake-up is this call's next WRITE_DONE. That wake-up
/// is only meaningful because a host eligible to ask holds nothing. AK_STATUS_BUDGET_BUSY is the
/// runtime-wide ceiling, and has no single event announcing room: poll ak_runtime_memory_usage.
/// AK_STATUS_MESSAGE_TOO_LARGE is permanent. On every refusal no buffer is lent and `*out` is
/// untouched.
///
/// # Safety
///
/// `out` must be writable.
#[no_mangle]
pub unsafe extern "C" fn ak_get_call_buffer(
    call: ak_handle,
    len: usize,
    out: *mut ak_buffer,
) -> ak_status {
    guard(|| unsafe { observe(tables::calls(), call, out, |found| found.lend(len)) })
}

/// Commits a lent buffer as the next message. Ownership passes back to this library.
///
/// AK_EVENT_WRITE_DONE settles an accepted send and frees its slot from the moment the event is
/// emitted, not when the callback returns - so a host woken by it may ask for a buffer from inside
/// the callback. It says nothing about the network: the message may have been written, or
/// abandoned because the call was cancelled. It arrives exactly once per accepted send, in send
/// order, and always before the terminal.
///
/// # Safety
///
/// `buffer` must be one this call lent and the host has not given back.
#[no_mangle]
pub unsafe extern "C" fn ak_call_send_message(call: ak_handle, buffer: ak_buffer) -> ak_status {
    guard(|| {
        let Some(lent) = (unsafe { call::take_lent(buffer.owner) }) else {
            return ak_status::AK_STATUS_INVALID_ARG;
        };
        let Some(found) = tables::calls().get(call) else {
            return call::keep(lent, ak_status::AK_STATUS_HANDLE_STALE);
        };
        // The buffer names its own call, so a handle that names another one is the host's
        // mistake and not this library's to resolve.
        if !Arc::ptr_eq(lent.call(), &found) {
            return call::keep(lent, ak_status::AK_STATUS_INVALID_ARG);
        }
        found.commit(lent)
    })
}

/// Gives a lent buffer back unused. Legal on a cancelled or terminal call: it is the only exit for
/// a buffer whose send is refused, and the call is not reclaimed until it happens.
///
/// Takes no call handle: the buffer determines its call. A refused ak_call_send_message therefore
/// leaves the buffer with the host, exactly as it was lent.
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
        Arc::clone(lent.call()).give_back(lent);
    });
}

/// Signals end of sending. No ak_call_send_message after this: a send that comes after the end,
/// and a second end, answer AK_STATUS_INVALID_STATE.
///
/// An end that comes while an ak_call_send_message has not yet returned on another thread - from
/// inside that send's own AK_EVENT_WRITE_DONE, which may arrive first - waits for it to return, so
/// it never goes ahead of a message the host has been told left.
#[no_mangle]
pub extern "C" fn ak_call_end_send(call: ak_handle) -> ak_status {
    guard(|| match tables::calls().get(call) {
        None => ak_status::AK_STATUS_HANDLE_STALE,
        Some(found) => found.end_send(),
    })
}

/// Cancels the call, which then reaches a terminal - carrying CANCELLED, unless the peer's own
/// status was already in.
///
/// Asynchronous: the request takes effect when the call's task observes it, so callbacks already
/// committed may still arrive after this returns, and a status the peer had already sent is the
/// one delivered. INITIAL_METADATA is never skipped.
#[no_mangle]
pub extern "C" fn ak_call_cancel(call: ak_handle) -> ak_status {
    guard(|| match tables::calls().get(call) {
        None => ak_status::AK_STATUS_HANDLE_STALE,
        Some(found) => {
            found.cancel();
            ak_status::AK_STATUS_OK
        }
    })
}

/// What the call still owes. Purely observational; it is legal never to call it. It exists because
/// without a release downcall nothing reports a forgotten ak_return_call_buffer synchronously.
///
/// # Safety
///
/// `out` must be writable.
#[no_mangle]
pub unsafe extern "C" fn ak_call_debt_of(call: ak_handle, out: *mut ak_call_debt) -> ak_status {
    guard(|| unsafe { observe(tables::calls(), call, out, |found| Ok(found.debt())) })
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
/// At most one non-consumed payload per call by default: while the host owes it, the runtime
/// withholds the next data callback. Only a terminal still goes out with the credit spent.
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
