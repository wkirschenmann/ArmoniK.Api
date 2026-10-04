// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use armonik_transport_ffi::*;
use support::host::*;
use support::{blob, flaky_seen, TestServer, ECHO, FAIL, FLAKY, SLOW};

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

    assert_eq!(
        unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    host.recorder.await_terminal();

    fixture.close();
}

#[test]
fn a_second_runtime_is_refused_while_the_first_is_alive() {
    let host = Host::start();

    let (status, second) = try_create_runtime(0, std::ptr::null_mut());

    assert_eq!(status, ak_status::AK_STATUS_INVALID_STATE);
    assert_eq!(second, AK_HANDLE_NONE, "a refusal leaves *out as it was");

    drop(host);

    let next = Host::start();
    assert_eq!(
        ak_runtime_status(next.runtime),
        ak_runtime_state::AK_RUNTIME_RUNNING
    );
}

/// The head event says where the head came from, in its status_code: the peer's headers, a
/// response that delivered none, or no response at all.
#[test]
fn a_head_event_says_where_the_head_came_from() {
    let server = TestServer::start();
    let host = Host::start();
    let served = host.channel(&server.endpoint);
    // A port bound and let go, so nothing listens on it when the call dials.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("an ephemeral port")
        .local_addr()
        .expect("its address")
        .port();
    // No retry: the subject is the head of the one attempt that fails to dial.
    let unreachable = host.channel_with(
        &format!("http://127.0.0.1:{port}"),
        r#"{"Retry":{"MaxAttempts":1}}"#,
    );

    for (what, channel, method, origin) in [
        ("headers", served, ECHO, ak_head_origin::AK_HEAD_RECEIVED),
        (
            "a Trailers-Only error",
            served,
            FAIL,
            ak_head_origin::AK_HEAD_TRAILERS_ONLY,
        ),
        (
            "no peer",
            unreachable,
            ECHO,
            ak_head_origin::AK_HEAD_NO_RESPONSE,
        ),
    ] {
        let heads = || {
            host.recorder
                .kinds()
                .iter()
                .filter(|kind| **kind == ak_event_kind::AK_EVENT_INITIAL_METADATA)
                .count()
        };
        let before = heads();
        let call = start_call(channel, method, &blob(&[]));
        // Nothing is lent to a call with no peer: it may have ended before the lend.
        if origin == ak_head_origin::AK_HEAD_NO_RESPONSE {
            let _ = unsafe { ak_call_end_send(call, std::ptr::null_mut()) };
        } else {
            send_one(call, b"x");
        }
        support::poll_until(|| heads() == before + 1, || format!("{what}: no head yet"));

        let head = host
            .recorder
            .last_of(ak_event_kind::AK_EVENT_INITIAL_METADATA)
            .expect("the head just seen");
        assert_eq!(head.status_code, origin as i32, "{what}");
        if origin != ak_head_origin::AK_HEAD_RECEIVED {
            assert_eq!(head.payload, blob(&[]), "{what}: an empty head");
        }
        support::await_call_reclaimed(call);
    }

    ak_channel_release(served);
    ak_channel_release(unreachable);
    host.stop();
}

/// The default window lets a response's message through while its head is still held, so a host
/// does not have to give the head back before the message it arrived with.
#[test]
fn a_default_channel_delivers_the_message_while_the_head_is_held() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);

    host.recorder.hold_payloads();
    let call = start_call(channel, ECHO, &blob(&[]));
    send_one(call, b"hello");

    support::poll_until(
        || {
            host.recorder
                .kinds()
                .contains(&ak_event_kind::AK_EVENT_MESSAGE)
        },
        || format!("only {:?} with the head held", host.recorder.kinds()),
    );

    host.recorder.consume_all();
    host.recorder.await_terminal();
    host.recorder.consume_all();
    fixture.close();
}

/// A window of one, so that a call whose head is held is parked before its message.
const ONE_CREDIT: &str = r#"{"DeliveryCredits":1}"#;

#[test]
fn releasing_a_channel_drains_a_call_parked_on_a_delivery_credit() {
    let server = TestServer::start();
    let host = &Host::start();
    let channel = host.channel_with(&server.endpoint, ONE_CREDIT);

    host.recorder.hold_payloads();
    let call = start_call(channel, ECHO, &blob(&[]));
    send_one(call, b"hello");
    host.recorder.await_metadata();

    ak_channel_release(channel);

    let seen = host.recorder.await_terminal();

    assert_eq!(seen.status_code(), Some(CANCELLED));

    // Closed, and reclaimed: the host released it.
    support::poll_until(
        || ak_channel_status(channel) == ak_channel_state::AK_CHANNEL_NONE,
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
        ak_channel_state::AK_CHANNEL_NONE
    );

    host.stop();
}

/// A release cancels the calls of its channel and no other: two calls parked on a delivery credit,
/// one on each of two channels, and the one whose channel stays open goes on to its answer.
#[test]
fn releasing_a_channel_cancels_its_calls_and_none_of_another_channels() {
    let server = TestServer::start();
    let host = Host::start();
    let released = host.channel_with(&server.endpoint, ONE_CREDIT);
    let kept = host.channel_with(&server.endpoint, ONE_CREDIT);

    host.recorder.hold_payloads();
    let doomed = start_call(released, ECHO, &blob(&[]));
    let spared = start_call(kept, ECHO, &blob(&[]));
    send_one(doomed, b"doomed");
    send_one(spared, b"spared");
    let heads = || {
        host.recorder
            .kinds()
            .iter()
            .filter(|kind| **kind == ak_event_kind::AK_EVENT_INITIAL_METADATA)
            .count()
    };
    support::poll_until(|| heads() == 2, || format!("{} of the two heads", heads()));

    ak_channel_release(released);

    assert_eq!(
        host.recorder.await_terminal().status_code(),
        Some(CANCELLED)
    );
    // Polled: the terminal is recorded inside the callback, and the call marks it delivered once
    // the callback has returned.
    support::poll_until(
        || debt_of(doomed).terminal_delivered == 1,
        || format!("the released call is {:?}", debt_of(doomed)),
    );
    assert_eq!(
        debt_of(spared).terminal_delivered,
        0,
        "the other channel's call was cancelled too"
    );

    host.recorder.consume_all();
    let answered = host.recorder.await_messages(1);
    assert_eq!(answered.message_payloads(), vec![b"spared".to_vec()]);
    support::poll_until(
        || debt_of(spared).terminal_delivered == 1,
        || format!("the kept call is {:?}", debt_of(spared)),
    );
    assert_eq!(
        host.recorder
            .last_of(ak_event_kind::AK_EVENT_STATUS)
            .map(|terminal| terminal.status_code),
        Some(0),
        "the kept call's answer"
    );

    host.recorder.consume_all();
    support::await_call_reclaimed(doomed);
    support::await_call_reclaimed(spared);
    ak_channel_release(kept);
    host.stop();
}

/// Once the sending has ended, a send and a second end are both refused, and the refused send's
/// buffer is still the host's to give back.
///
/// The answer is held on its delivery credit, so the call stays live and each refusal is the
/// ended sending's rather than a finished call's.
#[test]
fn nothing_is_sent_after_the_sending_has_ended() {
    let server = TestServer::start();
    let host = &Host::start();
    let channel = host.channel_with(&server.endpoint, ONE_CREDIT);
    host.recorder.hold_payloads();
    let call = start_call(channel, ECHO, &blob(&[]));

    send_one(call, b"hello");
    host.recorder.await_write_done();
    host.recorder.await_metadata();
    assert_eq!(
        unsafe { ak_call_end_send(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_INVALID_STATE
    );

    let (status, buffer) = lend(call, 1);
    assert_eq!(status, ak_status::AK_STATUS_OK, "the call is still live");
    assert_eq!(
        unsafe { ak_call_send_message(call, buffer, std::ptr::null_mut()) },
        ak_status::AK_STATUS_INVALID_STATE
    );
    unsafe { ak_return_call_buffer(buffer) };

    host.recorder.consume_all();
    host.recorder.await_terminal();
    host.recorder.consume_all();
    support::await_call_reclaimed(call);
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
    // this test is about what a released channel refuses, not about how far its drain has got:
    // closing while the call is on it, reclaimed once it has left.
    assert!(
        matches!(
            ak_channel_status(channel),
            ak_channel_state::AK_CHANNEL_CLOSING | ak_channel_state::AK_CHANNEL_NONE
        ),
        "the channel is {:?}",
        ak_channel_status(channel)
    );

    let (status, refused) = try_start_call(channel, ECHO, &[]);
    assert!(
        matches!(
            status,
            ak_status::AK_STATUS_INVALID_STATE | ak_status::AK_STATUS_HANDLE_STALE
        ),
        "{status:?}"
    );
    assert_eq!(refused, AK_HANDLE_NONE, "nothing was started");

    host.recorder.await_terminal();
    host.recorder.consume_all();
    support::await_call_reclaimed(first);
    support::poll_until(
        || ak_channel_status(channel) == ak_channel_state::AK_CHANNEL_NONE,
        || format!("the channel is {:?}", ak_channel_status(channel)),
    );

    host.stop();
}

/// With no call to wait for, the release closes the channel and reclaims its handle before it
/// returns.
#[test]
fn an_idle_channel_is_reclaimed_the_moment_it_is_released() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);

    assert_eq!(
        ak_channel_status(channel),
        ak_channel_state::AK_CHANNEL_OPEN
    );

    ak_channel_release(channel);

    assert_eq!(
        ak_channel_status(channel),
        ak_channel_state::AK_CHANNEL_NONE
    );

    let (status, call) = try_start_call(channel, ECHO, &blob(&[]));
    assert_eq!(status, ak_status::AK_STATUS_HANDLE_STALE);
    assert_eq!(call, AK_HANDLE_NONE, "a refusal leaves *out as it was");

    ak_channel_release(channel);
    assert_eq!(
        ak_channel_status(channel),
        ak_channel_state::AK_CHANNEL_NONE
    );

    host.stop();
}

/// The runtime's shutdown closes a channel without taking it from the host: it reads CLOSED and
/// refuses a call as a closed channel does, until the host releases it.
#[test]
fn a_channel_the_shutdown_closed_is_the_hosts_until_it_releases_it() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);

    host.stop();

    assert_eq!(
        ak_channel_status(channel),
        ak_channel_state::AK_CHANNEL_CLOSED
    );
    let (status, call) = try_start_call(channel, ECHO, &blob(&[]));
    assert_eq!(status, ak_status::AK_STATUS_INVALID_STATE);
    assert_eq!(call, AK_HANDLE_NONE);

    ak_channel_release(channel);
    assert_eq!(
        ak_channel_status(channel),
        ak_channel_state::AK_CHANNEL_NONE
    );
}

/// What arrives together is delivered together: the test server answers a unary call in one
/// write, so its head, its message and its status reach the host in one callback. Over several
/// calls, so that a machine that splits one answer's reads does not decide the test.
#[test]
fn a_unary_answer_that_arrives_together_comes_in_one_callback() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);

    let batched = (1..=20).any(|calls| {
        send_one(start_call(channel, ECHO, &[]), b"hello");
        host.recorder
            .await_terminals(calls)
            .last_call_data_callbacks()
            == 1
    });
    assert!(batched, "no answer of twenty came in one callback");
    fixture.close();
}

/// The flag reaches the engine: a server that answers a call declared one-response with two
/// messages ends it INTERNAL, the first delivered and the second never.
#[test]
fn a_one_response_call_answered_twice_ends_internal() {
    const INTERNAL: i32 = 13;
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);

    start_call_flagged(
        channel,
        "/raw/Sized",
        &blob(&[(b"x-sizes", b"3,3")]),
        AK_CALL_ONE_RESPONSE,
    );

    let seen = host.recorder.await_terminal();
    assert_eq!(
        seen.status_code(),
        Some(INTERNAL),
        "{}",
        seen.status_message()
    );
    assert_eq!(seen.message_payloads().len(), 1, "{seen:?}");
    fixture.close();
}

/// Payloads given back together are given back as one at a time would be: the call settles.
#[test]
fn payloads_given_back_in_one_downcall_settle_the_call() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    host.recorder.hold_payloads();

    let call = start_call(channel, ECHO, &[]);
    send_one(call, b"hello");
    host.recorder.await_terminal();
    assert_eq!(
        debt_of(call).payloads_owed,
        3,
        "the head, the message and the status"
    );

    host.recorder.consume_all_together();
    support::await_call_reclaimed(call);

    // Nothing to give back: a no-op, as it is with a null array.
    unsafe { ak_events_consumed(std::ptr::null(), 0) };
    host.recorder.stop_holding();
    fixture.close();
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
        unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_HANDLE_STALE,
        "a reclaimed call names nothing"
    );

    fixture.close();
}

/// A channel runs its calls on a thread of its own: a call's events all come from its channel's
/// thread, and another channel's from another.
#[test]
fn each_channel_runs_its_calls_on_a_thread_of_its_own() {
    let server = TestServer::start();
    let host = Host::start();
    let first = host.channel(&server.endpoint);
    let second = host.channel(&server.endpoint);

    send_one(start_call(first, ECHO, &[]), b"one");
    let on_first = host.recorder.await_terminal().threads();
    send_one(start_call(second, ECHO, &[]), b"two");
    let on_both = host.recorder.await_terminals(2).threads();
    let on_second = &on_both[on_first.len()..];

    let only = |threads: &[Option<String>]| {
        let name = threads[0].clone().expect("a named thread");
        assert!(name.starts_with("armonik-channel-"), "{threads:?}");
        assert!(
            threads
                .iter()
                .all(|thread| thread.as_deref() == Some(name.as_str())),
            "{threads:?}"
        );
        name
    };
    assert_ne!(only(&on_first), only(on_second));

    ak_channel_release(first);
    ak_channel_release(second);
    host.stop();
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
        unsafe { ak_call_send_message(call, first, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );

    assert_eq!(lend(call, 4).0, ak_status::AK_STATUS_SLOT_BUSY);

    host.recorder.await_write_done();
    let (status, second) = lend(call, 4);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    unsafe { ak_return_call_buffer(second) };

    assert_eq!(
        unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
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

    assert_eq!(
        unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
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
    assert_eq!(
        unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );

    assert_eq!(
        unsafe { ak_call_send_message(call, buffer, std::ptr::null_mut()) },
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
    assert_eq!(
        unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    host.recorder.await_terminal();
    // Polled: the terminal is recorded inside the callback, and the call marks it delivered once
    // the callback has returned.
    support::poll_until(
        || debt_of(call).terminal_delivered == 1,
        || format!("the call is {:?}", debt_of(call)),
    );

    let debt = debt_of(call);
    assert_eq!(debt.buffers_lent, 0);
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

    assert_eq!(
        unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
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
        unsafe { ak_runtime_begin_shutdown(host.runtime, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );

    host.recorder.await_shutdown();
    assert_eq!(
        host.recorder.shutdown_debt(),
        Some(ak_host_debt::AK_HOST_MUST_RETURN),
        "the host is still holding the call's payloads"
    );
    // Waited for: the event is recorded inside the callback, and STOPPED follows its return.
    host.await_state(ak_runtime_state::AK_RUNTIME_GRPC_STOPPED);
    assert_eq!(
        ak_runtime_status(host.runtime),
        ak_runtime_state::AK_RUNTIME_GRPC_STOPPED,
        "stopped is not quiescent while the host holds something"
    );
    assert_eq!(
        unsafe { ak_runtime_destroy(host.runtime, std::ptr::null_mut()) },
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

/// Level 1's RuntimeRelease publishes GRPC_STOPPED once the SHUTDOWN_COMPLETE callback has
/// returned, so the callback itself reads the runtime still stopping.
#[test]
fn the_shutdown_callback_reads_the_runtime_still_stopping() {
    let host = Host::start();
    host.stop();

    let shutdown = host
        .recorder
        .last_of(ak_event_kind::AK_EVENT_SHUTDOWN_COMPLETE)
        .expect("the shutdown was announced");
    assert_eq!(
        shutdown.runtime_state_inside,
        Some(ak_runtime_state::AK_RUNTIME_GRPC_STOPPING)
    );
}

fn start_options(method: &str) -> ak_call_start_options {
    ak_call_start_options {
        struct_size: std::mem::size_of::<ak_call_start_options>() as u32,
        version: 0,
        flags: 0,
        reserved: 0,
        method: ak_bytes_in {
            ptr: method.as_ptr(),
            len: method.len(),
        },
        metadata: ak_bytes_in {
            ptr: std::ptr::null(),
            len: 0,
        },
        timeout_ns: 0,
    }
}

/// gRPC's code, which is what the ABI carries in `status_code`.
const DEADLINE_EXCEEDED: i32 = 4;

/// The retry the options name is the engine's: a call that fails once answers on its second
/// attempt, which the host never sees.
#[test]
fn the_retry_options_reach_the_engine() {
    let server = TestServer::start();
    let host = Host::start();
    let channel = host.channel_with(
        &server.endpoint,
        r#"{"Retry":{"InitialBackoffSeconds":0.01,"MaxBackoffSeconds":0.05}}"#,
    );
    let metadata = blob(&[
        (b"x-flaky-key", b"through-the-abi"),
        (b"x-fail-times", b"1"),
    ]);
    let call = start_call(channel, FLAKY, &metadata);
    send_one(call, b"hello");

    let seen = host.recorder.await_terminal();
    assert_eq!(seen.status_code(), Some(0), "{}", seen.status_message());
    assert_eq!(seen.message_payloads(), vec![b"hello".to_vec()]);
    assert_eq!(flaky_seen("through-the-abi").len(), 2);
    ak_channel_release(channel);
    host.stop();
}

#[test]
fn an_eager_channel_is_connected_before_its_first_call_and_a_lazy_one_is_not() {
    let host = Host::start();
    let eager_server = TestServer::start();
    let lazy_server = TestServer::start();
    let eager = host.channel_with(&eager_server.endpoint, r#"{"ConnectEagerly":true}"#);
    let lazy = host.channel(&lazy_server.endpoint);

    support::poll_until(
        || eager_server.connections() == 1,
        || format!("the eager server saw {}", eager_server.connections()),
    );
    assert_eq!(lazy_server.connections(), 0, "the lazy channel dialled");

    let call = start_call(eager, ECHO, &blob(&[]));
    send_one(call, b"hello");
    let seen = host.recorder.await_terminal();
    assert_eq!(seen.status_code(), Some(0), "{}", seen.status_message());
    assert_eq!(
        eager_server.connections(),
        1,
        "the call took the eager session"
    );
    // Still none, a whole call later.
    assert_eq!(lazy_server.connections(), 0, "the lazy channel dialled");

    ak_channel_release(eager);
    ak_channel_release(lazy);
    host.stop();
}

/// A released channel closes its connection the way HTTP/2 closes one, rather than dropping it.
#[test]
fn a_released_channel_closes_its_connection_cleanly() {
    let host = Host::start();
    let server = TestServer::start();
    let channel = host.channel_with(&server.endpoint, r#"{"ConnectEagerly":true}"#);
    support::poll_until(
        || server.connections() == 1,
        || format!("the server saw {}", server.connections()),
    );

    ak_channel_release(channel);

    support::poll_until(
        || !server.goodbyes().is_empty(),
        || "the connection did not end".to_owned(),
    );
    assert_eq!(server.goodbyes(), [true], "the connection was dropped");
    host.stop();
}

/// gRPC's code, which is what the ABI carries in `status_code`.
const UNAVAILABLE: i32 = 14;

#[test]
fn an_eager_dial_that_fails_leaves_the_first_call_to_report_it() {
    let host = Host::start();
    let channel = host.channel_with(
        "http://127.0.0.1:1",
        r#"{"ConnectEagerly":true,"Retry":{"MaxAttempts":1}}"#,
    );

    let call = start_call(channel, ECHO, &blob(&[]));
    send_one(call, b"hello");
    let seen = host.recorder.await_terminal();
    assert_eq!(
        seen.status_code(),
        Some(UNAVAILABLE),
        "{}",
        seen.status_message()
    );

    ak_channel_release(channel);
    host.stop();
}

/// The flag names a field this record does not reach, which would read as a deadline passed.
#[test]
fn a_deadline_flag_on_a_record_too_short_for_its_field_is_refused() {
    let host = Host::start();
    let channel = host.channel("http://127.0.0.1:1");
    let options = ak_call_start_options {
        struct_size: std::mem::offset_of!(ak_call_start_options, timeout_ns) as u32,
        flags: AK_CALL_HAS_DEADLINE,
        timeout_ns: 1_000_000_000,
        ..start_options(ECHO)
    };
    let (status, error, call) = start_with(channel, &options);
    assert_eq!(status, ak_status::AK_STATUS_INVALID_ARG);
    assert_eq!(error.kind, ak_error_kind::AK_ERROR_USAGE);
    assert_eq!(call, AK_HANDLE_NONE, "nothing was started");
    ak_channel_release(channel);
}

#[test]
fn a_deadline_the_record_states_ends_the_call_deadline_exceeded() {
    for timeout in [
        std::time::Duration::from_millis(200),
        std::time::Duration::ZERO,
    ] {
        let fixture = Host::connected();
        let (host, channel) = (&fixture.host, fixture.channel);
        let options = ak_call_start_options {
            flags: AK_CALL_HAS_DEADLINE,
            timeout_ns: timeout.as_nanos() as u64,
            ..start_options(SLOW)
        };
        let (status, _, _) = start_with(channel, &options);
        assert_eq!(status, ak_status::AK_STATUS_OK);

        let seen = host.recorder.await_terminal();
        assert_eq!(seen.status_code(), Some(DEADLINE_EXCEEDED), "{timeout:?}");
        fixture.close();
    }
}

/// A record that stops before `timeout_ns` is valid, and states no deadline.
#[test]
fn a_record_of_the_first_definition_is_read_as_one_with_no_deadline() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    let options = ak_call_start_options {
        struct_size: std::mem::offset_of!(ak_call_start_options, timeout_ns) as u32,
        ..start_options(ECHO)
    };
    let (status, _, call) = start_with(channel, &options);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    send_one(call, b"hello");

    let seen = host.recorder.await_terminal();
    assert_eq!(seen.status_code(), Some(0), "{}", seen.status_message());
    fixture.close();
}

/// `options` as a pointer to whatever record the test built around it.
fn start_with<T>(channel: ak_handle, options: &T) -> (ak_status, ak_error, ak_handle) {
    let mut call = AK_HANDLE_NONE;
    let mut error = std::mem::MaybeUninit::<ak_error>::zeroed();
    let status = unsafe {
        ak_call_start(
            channel,
            (options as *const T).cast(),
            std::ptr::null_mut(),
            &mut call,
            error.as_mut_ptr(),
        )
    };
    (status, unsafe { error.assume_init() }, call)
}

#[test]
fn a_record_shorter_than_its_first_definition_is_refused_rather_than_read() {
    let host = Host::start();
    let channel = host.channel("http://127.0.0.1:1");

    let options = ak_call_start_options {
        struct_size: std::mem::offset_of!(ak_call_start_options, timeout_ns) as u32 - 1,
        ..start_options(ECHO)
    };
    let (status, error, call) = start_with(channel, &options);

    assert_eq!(status, ak_status::AK_STATUS_INVALID_ARG);
    assert_eq!(error.kind, ak_error_kind::AK_ERROR_USAGE);
    assert_eq!(call, AK_HANDLE_NONE, "nothing was started");
    ak_channel_release(channel);
}

#[test]
fn a_record_one_field_longer_is_read_and_its_tail_ignored() {
    #[repr(C)]
    struct Longer {
        known: ak_call_start_options,
        unknown: u64,
    }

    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    let options = Longer {
        known: ak_call_start_options {
            struct_size: std::mem::size_of::<Longer>() as u32,
            ..start_options(ECHO)
        },
        unknown: u64::MAX,
    };
    let (status, _, call) = start_with(channel, &options);

    assert_eq!(status, ak_status::AK_STATUS_OK);
    assert_eq!(
        unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    host.recorder.await_terminal();
    fixture.close();
}

#[test]
fn a_record_that_sets_a_version_a_flag_or_a_reserved_field_is_refused() {
    let host = Host::start();
    let channel = host.channel("http://127.0.0.1:1");

    for (field, options) in [
        (
            "version",
            ak_call_start_options {
                version: 1,
                ..start_options(ECHO)
            },
        ),
        (
            "flags",
            ak_call_start_options {
                flags: 1 << 31,
                ..start_options(ECHO)
            },
        ),
        (
            "reserved",
            ak_call_start_options {
                reserved: 1,
                ..start_options(ECHO)
            },
        ),
    ] {
        let (status, error, call) = start_with(channel, &options);
        assert_eq!(status, ak_status::AK_STATUS_INVALID_ARG, "{field}");
        assert_eq!(error.kind, ak_error_kind::AK_ERROR_USAGE, "{field}");
        assert_eq!(call, AK_HANDLE_NONE, "{field}: nothing was started");
    }
    ak_channel_release(channel);
}

#[test]
fn a_runtime_config_carries_the_same_head_as_call_options() {
    // Refused before the one-at-a-time rule is asked, so no other test's runtime enters it.
    let config = ak_runtime_config {
        struct_size: std::mem::size_of::<ak_runtime_config>() as u32,
        version: 1,
        flags: 0,
        reserved: 0,
        memory_ceiling: 0,
        memory_hard_ceiling: 0,
    };
    let mut runtime = AK_HANDLE_NONE;
    let status = unsafe {
        ak_runtime_create(
            &config,
            Some(support::on_event),
            std::ptr::null_mut(),
            &mut runtime,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(status, ak_status::AK_STATUS_INVALID_ARG);
    assert_eq!(runtime, AK_HANDLE_NONE);
}

#[test]
fn a_token_naming_nothing_is_refused_rather_than_dereferenced() {
    assert_eq!(
        unsafe { ak_call_cancel(AK_HANDLE_NONE, std::ptr::null_mut()) },
        ak_status::AK_STATUS_HANDLE_STALE
    );
    assert_eq!(
        unsafe { ak_call_end_send(u64::MAX, std::ptr::null_mut()) },
        ak_status::AK_STATUS_HANDLE_STALE
    );
    assert_eq!(
        unsafe { ak_runtime_begin_shutdown(u64::MAX, std::ptr::null_mut()) },
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
        unsafe { ak_runtime_begin_shutdown(host.runtime, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );

    let (status, call) = try_start_call(channel, ECHO, &[]);
    assert_eq!(status, ak_status::AK_STATUS_INVALID_STATE);
    assert_eq!(call, AK_HANDLE_NONE, "nothing was started");

    let endpoint = b"http://127.0.0.1:1";
    let json = b"{}";
    let mut opened = AK_HANDLE_NONE;
    let status = unsafe {
        ak_channel_create(
            host.runtime,
            ak_bytes_in {
                ptr: endpoint.as_ptr(),
                len: endpoint.len(),
            },
            ak_bytes_in {
                ptr: json.as_ptr(),
                len: json.len(),
            },
            &mut opened,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(status, ak_status::AK_STATUS_INVALID_STATE);
    assert_eq!(opened, AK_HANDLE_NONE, "and no channel either");

    host.await_state(ak_runtime_state::AK_RUNTIME_QUIESCENT);
}

/// A closed channel the host has not released stays in the tables until the runtime is
/// destroyed: the destroy is what takes it out, and its handle names nothing from then on. One the
/// host released has already gone.
#[test]
fn destroying_a_runtime_leaves_none_of_its_handles_naming_anything() {
    let server = TestServer::start();
    let host = Host::start();
    let released = host.channel(&server.endpoint);
    let kept = host.channel(&server.endpoint);
    let call = start_call(kept, ECHO, &[]);

    ak_channel_release(released);
    host.stop();

    assert_eq!(
        ak_channel_status(released),
        ak_channel_state::AK_CHANNEL_NONE
    );
    assert_eq!(ak_channel_status(kept), ak_channel_state::AK_CHANNEL_CLOSED);

    let runtime = host.runtime;
    drop(host);

    assert_eq!(
        ak_runtime_status(runtime),
        ak_runtime_state::AK_RUNTIME_NONE
    );
    for channel in [released, kept] {
        assert_eq!(
            ak_channel_status(channel),
            ak_channel_state::AK_CHANNEL_NONE
        );
    }
    assert_eq!(
        unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_HANDLE_STALE
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

/// The downcall that pays an ended call's last debt reclaims it: once the host has given back
/// every payload, the handle is stale, with nothing left to wait for.
#[test]
fn the_downcall_that_pays_the_last_debt_reclaims_the_call() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    host.recorder.hold_payloads();

    let call = start_call(channel, ECHO, &[]);
    send_one(call, b"hello");
    host.recorder.await_terminals(1);
    // Out of its terminal callback, so that the payloads are the call's last debt.
    support::poll_until(
        || debt_of(call).callbacks_in_flight == 0,
        || "the terminal callback did not return".to_owned(),
    );

    host.recorder.consume_all_together();
    assert_eq!(
        unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_HANDLE_STALE
    );

    host.recorder.stop_holding();
    fixture.close();
}
