// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use armonik_transport_ffi::*;
use support::host::*;
use support::{blob, TestServer, ECHO, FAIL, SLOW};

/// gRPC's code, which is what the ABI carries in `status_code`.
const CANCELLED: i32 = 1;

#[test]
fn a_second_buffer_while_the_first_is_still_held_is_a_host_bug() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call(channel, ECHO, &blob(&[]));

    let (first, buffer) = lend(call, 8);
    assert_eq!(first, ak_status::AK_STATUS_OK);

    let (second, _) = lend(call, 8);
    assert_eq!(second, ak_status::AK_STATUS_INVALID_STATE);

    unsafe { ak_return_call_buffer(buffer) };

    let (third, third_buffer) = lend(call, 8);
    assert_eq!(third, ak_status::AK_STATUS_OK);
    unsafe { ak_return_call_buffer(third_buffer) };

    assert_eq!(ak_call_cancel(call), ak_status::AK_STATUS_OK);
    host.recorder.await_terminal();

    fixture.close();
}

#[test]
fn a_second_runtime_is_refused_while_the_first_is_alive() {
    let host = Host::start();

    let (status, second) = try_create_runtime(1, 0, std::ptr::null_mut());

    assert_eq!(status, ak_status::AK_STATUS_INVALID_STATE);
    assert_eq!(second, AK_HANDLE_NONE, "a refusal leaves *out as it was");

    drop(host);

    let next = Host::start();
    assert_eq!(
        ak_runtime_status(next.runtime),
        ak_runtime_state::AK_RUNTIME_RUNNING
    );
}

#[test]
fn releasing_a_channel_drains_a_call_parked_on_a_delivery_credit() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);

    host.recorder.hold_payloads();
    let call = start_call(channel, ECHO, &blob(&[]));
    send_one(call, b"hello");
    host.recorder.await_metadata();

    ak_channel_release(channel);

    let seen = host.recorder.await_terminal();

    assert_eq!(seen.status_code(), Some(CANCELLED));

    support::poll_until(
        || ak_channel_status(channel) == ak_channel_state::AK_CHANNEL_CLOSED,
        || format!("the channel is {:?}", ak_channel_status(channel)),
    );

    // No MESSAGE, because the credit the metadata took never came back: that is the call being
    // parked in `deliver`, which is the case this test exists for. `payloads_owed > 0` would say
    // nothing - `hold_payloads` is on, so the terminal's own payload is owed whatever happened.
    assert_eq!(
        seen.data_kinds(),
        vec![
            ak_event_kind::AK_EVENT_INITIAL_METADATA,
            ak_event_kind::AK_EVENT_STATUS
        ],
        "the reader was parked on a credit, so no message was delivered"
    );

    host.recorder.consume_all();
    support::await_call_reclaimed(call);
    assert_eq!(
        ak_channel_status(channel),
        ak_channel_state::AK_CHANNEL_CLOSED
    );

    host.stop();
}

#[test]
fn a_closing_channel_takes_no_new_call() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);

    host.recorder.hold_payloads();
    let first = start_call(channel, ECHO, &blob(&[]));
    send_one(first, b"hello");
    host.recorder.await_metadata();

    ak_channel_release(channel);
    // Either, because the call may reach its terminal between the release and this read, and
    // this test is about what a released channel refuses, not about how far its drain has got.
    assert!(
        matches!(
            ak_channel_status(channel),
            ak_channel_state::AK_CHANNEL_CLOSING | ak_channel_state::AK_CHANNEL_CLOSED
        ),
        "the channel is {:?}",
        ak_channel_status(channel)
    );

    let (status, refused) = try_start_call(channel, ECHO, &[]);
    assert_eq!(status, ak_status::AK_STATUS_INVALID_STATE);
    assert_eq!(refused, AK_HANDLE_NONE, "nothing was started");

    host.recorder.await_terminal();
    host.recorder.consume_all();
    support::await_call_reclaimed(first);
    support::poll_until(
        || ak_channel_status(channel) == ak_channel_state::AK_CHANNEL_CLOSED,
        || format!("the channel is {:?}", ak_channel_status(channel)),
    );

    host.stop();
}

#[test]
fn an_idle_channel_is_closed_the_moment_it_is_released() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);

    assert_eq!(
        ak_channel_status(channel),
        ak_channel_state::AK_CHANNEL_OPEN
    );

    ak_channel_release(channel);

    assert_eq!(
        ak_channel_status(channel),
        ak_channel_state::AK_CHANNEL_CLOSED
    );

    let (status, call) = try_start_call(channel, ECHO, &blob(&[]));
    assert_eq!(status, ak_status::AK_STATUS_INVALID_STATE);
    assert_eq!(call, AK_HANDLE_NONE, "a refusal leaves *out as it was");

    ak_channel_release(channel);
    assert_eq!(
        ak_channel_status(channel),
        ak_channel_state::AK_CHANNEL_CLOSED
    );

    host.stop();
}

#[test]
fn a_unary_call_through_the_abi_reaches_a_grpc_server_and_comes_back() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);

    let call = start_call(channel, ECHO, &blob(&[(b"x-request", b"ping")]));
    send_one(call, b"hello");

    let seen = host.recorder.await_terminal();

    // The order the header promises for every call, head first and terminal last, which is what
    // lets a host size its ring and know when it is done without inspecting what it holds.
    assert_eq!(
        seen.data_kinds(),
        vec![
            ak_event_kind::AK_EVENT_INITIAL_METADATA,
            ak_event_kind::AK_EVENT_MESSAGE,
            ak_event_kind::AK_EVENT_STATUS,
        ],
        "{seen:?}"
    );
    assert_eq!(
        seen.kinds()
            .iter()
            .filter(|kind| **kind == ak_event_kind::AK_EVENT_WRITE_DONE)
            .count(),
        1,
        "one acquittal for the one accepted send"
    );
    assert_eq!(seen.message_payloads(), vec![b"hello".to_vec()]);
    assert_eq!(seen.status_code(), Some(0));
    assert_eq!(
        seen.initial_metadata().get(&b"x-echoed"[..]),
        Some(&b"ping".to_vec()),
        "the request metadata crossed the ABI and its answer came back"
    );

    support::await_call_reclaimed(call);
    assert_eq!(
        ak_call_cancel(call),
        ak_status::AK_STATUS_HANDLE_STALE,
        "a reclaimed call names nothing"
    );

    fixture.close();
}

#[test]
fn a_refused_method_comes_back_as_its_status_behind_a_synthesized_metadata_event() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);

    let call = start_call(channel, FAIL, &[]);
    send_one(call, b"x");

    let seen = host.recorder.await_terminal();

    assert_eq!(
        seen.data_kinds()[0],
        ak_event_kind::AK_EVENT_INITIAL_METADATA
    );
    assert!(seen.initial_metadata().is_empty());
    // Every data event is given back through `ak_event_consumed`, empty ones included: a payload
    // without an owner would be one the host cannot return and the runtime would wait for.
    assert!(seen.first_data_event_was_owned(), "empty is not unowned");
    assert_eq!(seen.status_code(), Some(7), "PERMISSION_DENIED");
    assert_eq!(seen.status_message(), "not for you");

    fixture.close();
}

#[test]
fn the_send_window_refuses_a_second_buffer_until_a_write_is_acquitted() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call(channel, SLOW, &[]);

    let (status, first) = lend(call, 4);
    assert_eq!(status, ak_status::AK_STATUS_OK);

    assert_eq!(lend(call, 4).0, ak_status::AK_STATUS_INVALID_STATE);

    assert_eq!(
        unsafe { ak_call_send_message(call, first) },
        ak_status::AK_STATUS_OK
    );

    assert_eq!(lend(call, 4).0, ak_status::AK_STATUS_SLOT_BUSY);

    host.recorder.await_write_done();
    let (status, second) = lend(call, 4);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    unsafe { ak_return_call_buffer(second) };

    assert_eq!(ak_call_cancel(call), ak_status::AK_STATUS_OK);
    host.recorder.await_terminal();
    fixture.close();
}

#[test]
fn a_length_no_frame_can_carry_is_refused_and_charges_nothing() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call(channel, SLOW, &[]);

    // Past this library's own ceiling on both widths, which is what a host that configured
    // none gets: the four-byte gRPC prefix on a 64-bit target, and half the address space on a
    // 32-bit one, where the allocator is the tighter of the two.
    assert_eq!(
        lend(call, usize::MAX).0,
        ak_status::AK_STATUS_MESSAGE_TOO_LARGE
    );

    // A refusal that charged the ledger or spent the window permit would leave the call unable to
    // settle and the runtime unable to quiesce, so the failure would arrive as a destroy that never
    // succeeds rather than as the refusal it is.
    assert_eq!(memory_usage(host.runtime).bytes_used, 0);
    assert_eq!(debt_of(call).buffers_lent, 0);

    let (status, buffer) = lend(call, 8);
    assert_eq!(status, ak_status::AK_STATUS_OK, "the window is intact");
    unsafe { ak_return_call_buffer(buffer) };

    assert_eq!(ak_call_cancel(call), ak_status::AK_STATUS_OK);
    host.recorder.await_terminal();
    fixture.close();
}

#[test]
fn a_buffer_a_refused_send_hands_back_is_the_host_to_return() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call(channel, SLOW, &[]);

    let (status, buffer) = lend(call, 4);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    assert_eq!(ak_call_cancel(call), ak_status::AK_STATUS_OK);

    assert_eq!(
        unsafe { ak_call_send_message(call, buffer) },
        ak_status::AK_STATUS_INVALID_STATE
    );
    unsafe { ak_return_call_buffer(buffer) };

    host.recorder.await_terminal();
    fixture.close();
}

#[test]
fn a_call_reports_what_the_host_owes_it() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    host.recorder.hold_payloads();
    let call = start_call(channel, SLOW, &[]);

    let (_, buffer) = lend(call, 8);
    let debt = debt_of(call);
    assert_eq!(debt.buffers_lent, 1);
    assert_eq!(debt.terminal_delivered, 0);

    unsafe { ak_return_call_buffer(buffer) };
    assert_eq!(ak_call_cancel(call), ak_status::AK_STATUS_OK);
    host.recorder.await_terminal();

    let debt = debt_of(call);
    assert_eq!(debt.buffers_lent, 0);
    assert_eq!(debt.terminal_delivered, 1);
    assert!(
        debt.payloads_owed >= 1,
        "the host is holding what it was given: {debt:?}"
    );

    host.recorder.consume_all();
    fixture.close();
}

#[test]
fn the_ceiling_refuses_what_will_never_fit_apart_from_what_does_not_fit_yet() {
    let fixture = Connected::with_ceiling(64);
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call(channel, ECHO, &[]);

    assert_eq!(lend(call, 65).0, ak_status::AK_STATUS_MESSAGE_TOO_LARGE);

    let (status, buffer) = lend(call, 40);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    let usage = memory_usage(host.runtime);
    assert_eq!(
        usage,
        ak_memory_usage {
            bytes_used: 40,
            ceiling: 64
        }
    );

    unsafe { ak_return_call_buffer(buffer) };
    let usage = memory_usage(host.runtime);
    assert_eq!(usage.bytes_used, 0);

    assert_eq!(ak_call_cancel(call), ak_status::AK_STATUS_OK);
    host.recorder.await_terminal();
    fixture.close();
}

#[test]
fn a_runtime_the_host_still_owes_says_so_and_reaches_quiescence_when_it_is_paid() {
    let server = TestServer::start();
    let host = Host::start();
    host.recorder.hold_payloads();
    let channel = host.channel(&server.endpoint);

    let call = start_call(channel, ECHO, &[]);
    send_one(call, b"held");
    host.recorder.await_metadata();

    ak_channel_release(channel);
    assert_eq!(
        ak_runtime_begin_shutdown(host.runtime),
        ak_status::AK_STATUS_OK
    );

    host.recorder.await_shutdown();
    assert_eq!(
        host.recorder.shutdown_debt(),
        Some(ak_host_debt::AK_HOST_MUST_RETURN),
        "the host is still holding the call's payloads"
    );
    assert_eq!(
        ak_runtime_status(host.runtime),
        ak_runtime_state::AK_RUNTIME_GRPC_STOPPED,
        "stopped is not quiescent while the host holds something"
    );
    assert_eq!(
        ak_runtime_destroy(host.runtime),
        ak_status::AK_STATUS_INVALID_STATE,
        "destroying before quiescence is refused"
    );

    host.recorder.consume_all();
    host.await_state(ak_runtime_state::AK_RUNTIME_QUIESCENT);

    // Quiescence is a thread having gone, not a task having reached a line: the last event is
    // emitted by a thread of its own, which then shuts tokio down and finishes, and the status
    // answers QUIESCENT by asking whether that thread has finished. The name is what pins it -
    // no tokio worker is called this.
    assert_eq!(
        host.recorder
            .last_of(ak_event_kind::AK_EVENT_RESOURCES_RELEASED)
            .expect("the second event went out")
            .on_thread
            .as_deref(),
        Some("armonik-teardown")
    );

    assert!(
        host.recorder
            .kinds()
            .contains(&ak_event_kind::AK_EVENT_RESOURCES_RELEASED),
        "the second event is owed and arrives"
    );
}

#[test]
fn a_runtime_that_owes_nothing_gets_one_shutdown_event_and_no_second() {
    let host = Host::start();
    host.stop();

    let kinds = host.recorder.kinds();
    assert_eq!(kinds, vec![ak_event_kind::AK_EVENT_SHUTDOWN_COMPLETE]);
    assert_eq!(
        host.recorder.shutdown_debt(),
        Some(ak_host_debt::AK_HOST_NOTHING_TO_RETURN)
    );
}

#[test]
fn a_struct_of_an_unknown_size_is_refused_rather_than_read() {
    let host = Host::start();
    let channel = host.channel("http://127.0.0.1:1");

    let options = ak_call_start_options {
        struct_size: 7,
        method: ak_bytes_in {
            ptr: ECHO.as_ptr(),
            len: ECHO.len(),
        },
        metadata: ak_bytes_in {
            ptr: std::ptr::null(),
            len: 0,
        },
    };
    let mut call = AK_HANDLE_NONE;
    let status = unsafe { ak_call_start(channel, &options, std::ptr::null_mut(), &mut call) };

    assert_eq!(status, ak_status::AK_STATUS_INVALID_ARG);
    assert_eq!(call, AK_HANDLE_NONE, "nothing was started");
    ak_channel_release(channel);
}

#[test]
fn a_token_naming_nothing_is_refused_rather_than_dereferenced() {
    assert_eq!(
        ak_call_cancel(AK_HANDLE_NONE),
        ak_status::AK_STATUS_HANDLE_STALE
    );
    assert_eq!(
        ak_call_end_send(u64::MAX),
        ak_status::AK_STATUS_HANDLE_STALE
    );
    assert_eq!(
        ak_runtime_begin_shutdown(u64::MAX),
        ak_status::AK_STATUS_HANDLE_STALE
    );
    ak_channel_release(u64::MAX);
}

/// The two entry points that answer with a state rather than a status say NONE for a handle they
/// do not know, which is the same rule the ones above keep with HANDLE_STALE.
///
/// QUIESCENT would be the wrong answer and not merely an unhelpful one: it is the value that
/// permits `ak_runtime_destroy` and unloading the library, so a host that passed a channel handle
/// where the runtime's goes would read a licence to unload while a runtime is running.
#[test]
fn a_state_is_asked_of_a_handle_this_library_knows_or_it_is_none() {
    let fixture = Host::connected();
    let (runtime, channel) = (fixture.host.runtime, fixture.channel);

    assert_eq!(
        ak_runtime_status(AK_HANDLE_NONE),
        ak_runtime_state::AK_RUNTIME_NONE
    );
    assert_eq!(
        ak_runtime_status(channel),
        ak_runtime_state::AK_RUNTIME_NONE,
        "a channel handle names no runtime"
    );
    assert_eq!(
        ak_channel_status(runtime),
        ak_channel_state::AK_CHANNEL_NONE,
        "and a runtime handle names no channel"
    );

    fixture.close();
    drop(fixture);

    assert_eq!(
        ak_runtime_status(runtime),
        ak_runtime_state::AK_RUNTIME_NONE,
        "destroy reclaims the handle, so it names nothing after it"
    );
}

/// The start gate, read while the runtime is still stopping rather than after it stopped.
///
/// `begin_shutdown` stores GRPC_STOPPING before it spawns anything, so this is deterministic, and
/// it is the only moment the gate is what refuses: waiting for QUIESCENT first closes the channel
/// and hands the refusal to the transport, which would hold with the gate deleted.
#[test]
fn nothing_starts_on_a_runtime_that_has_begun_stopping() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);

    assert_eq!(
        ak_runtime_begin_shutdown(host.runtime),
        ak_status::AK_STATUS_OK
    );

    let (status, call) = try_start_call(channel, ECHO, &[]);
    assert_eq!(status, ak_status::AK_STATUS_INVALID_STATE);
    assert_eq!(call, AK_HANDLE_NONE, "nothing was started");

    let json = br#"{"endpoint":"http://127.0.0.1:1"}"#;
    let mut opened = AK_HANDLE_NONE;
    let status = unsafe {
        ak_channel_create(
            host.runtime,
            ak_bytes_in {
                ptr: json.as_ptr(),
                len: json.len(),
            },
            &mut opened,
        )
    };
    assert_eq!(status, ak_status::AK_STATUS_INVALID_STATE);
    assert_eq!(opened, AK_HANDLE_NONE, "and no channel either");

    host.await_state(ak_runtime_state::AK_RUNTIME_QUIESCENT);
}

#[test]
fn a_channel_released_before_the_runtime_stops_is_closed_by_the_time_it_does() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);

    host.stop();

    assert_eq!(
        ak_channel_status(channel),
        ak_channel_state::AK_CHANNEL_CLOSED
    );
}

#[test]
fn the_abi_version_is_the_one_the_header_carries() {
    assert_eq!(ak_abi_version(), AK_ABI_VERSION);
}

#[test]
fn consuming_an_unowned_payload_is_a_no_op() {
    unsafe {
        ak_event_consumed(ak_bytes {
            ptr: std::ptr::null(),
            len: 0,
            owner: std::ptr::null_mut(),
        })
    };
}
