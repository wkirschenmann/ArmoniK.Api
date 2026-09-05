mod support;

use std::sync::{Mutex, MutexGuard};

use std::ffi::c_void;

use armonik_transport_ffi::*;
use support::{blob, empty_buffer, Recorder, TestServer, ECHO, FAIL, SLOW};

/// gRPC's code, which is what the ABI carries in `status_code`.
const CANCELLED: i32 = 1;

struct Connected {
    channel: ak_handle,
    host: Host,
    _server: TestServer,
}

impl Connected {
    fn with_ceiling(memory_ceiling: u64) -> Self {
        let server = TestServer::start();
        let host = Host::with_ceiling(memory_ceiling);
        let channel = host.channel(&server.endpoint);
        Self {
            channel,
            host,
            _server: server,
        }
    }

    fn close(&self) {
        ak_channel_release(self.channel);
        self.host.stop();
    }
}

static ONE_RUNTIME: Mutex<()> = Mutex::new(());

struct Host {
    runtime: ak_handle,
    recorder: Box<Recorder>,
    _turn: MutexGuard<'static, ()>,
}

impl Host {
    fn start() -> Self {
        Self::with_ceiling(0)
    }

    fn with_ceiling(memory_ceiling: u64) -> Self {
        let turn = ONE_RUNTIME.lock().unwrap_or_else(|held| held.into_inner());
        let mut recorder = Box::new(Recorder::default());
        let (status, runtime) = try_create_runtime(
            2,
            memory_ceiling,
            recorder.as_mut() as *mut Recorder as *mut c_void,
        );
        assert_eq!(status, ak_status::AK_STATUS_OK);
        assert_eq!(
            ak_runtime_status(runtime),
            ak_runtime_state::AK_RUNTIME_RUNNING
        );

        Self {
            runtime,
            recorder,
            _turn: turn,
        }
    }

    fn connected() -> Connected {
        Connected::with_ceiling(0)
    }

    fn channel(&self, endpoint: &str) -> ak_handle {
        let json = format!(r#"{{"endpoint":"{endpoint}"}}"#);
        let mut channel = AK_HANDLE_NONE;
        let status = unsafe {
            ak_channel_create(
                self.runtime,
                ak_bytes_in {
                    ptr: json.as_ptr(),
                    len: json.len(),
                },
                &mut channel,
            )
        };
        assert_eq!(status, ak_status::AK_STATUS_OK, "{json}");
        channel
    }

    fn stop(&self) {
        assert_eq!(
            ak_runtime_begin_shutdown(self.runtime),
            ak_status::AK_STATUS_OK
        );
        self.await_state(ak_runtime_state::AK_RUNTIME_QUIESCENT);
    }

    fn await_state(&self, wanted: ak_runtime_state) {
        support::poll_until(
            || ak_runtime_status(self.runtime) == wanted,
            || {
                format!(
                    "the runtime is {:?} and not {wanted:?}; it still holds {} events",
                    ak_runtime_status(self.runtime),
                    self.recorder.len()
                )
            },
        );
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        if ak_runtime_status(self.runtime) != ak_runtime_state::AK_RUNTIME_QUIESCENT {
            self.stop();
        }
        assert_eq!(ak_runtime_destroy(self.runtime), ak_status::AK_STATUS_OK);
    }
}

fn lend(call: ak_handle, len: usize) -> (ak_status, ak_buffer) {
    let mut buffer = empty_buffer();
    let status = unsafe { ak_get_call_buffer(call, len, &mut buffer) };
    if status != ak_status::AK_STATUS_OK {
        assert!(buffer.owner.is_null(), "a refusal leaves *out as it was");
    }
    (status, buffer)
}

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

    let debt = debt_of(call);
    assert!(
        debt.payloads_owed > 0,
        "the channel closed with nothing outstanding, so this proves nothing: {debt:?}"
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
    assert_eq!(
        ak_channel_status(channel),
        ak_channel_state::AK_CHANNEL_CLOSING
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

fn send_one(call: ak_handle, message: &[u8]) {
    let (status, buffer) = lend(call, message.len());
    assert_eq!(status, ak_status::AK_STATUS_OK);
    assert_eq!(buffer.len, message.len());

    unsafe { std::ptr::copy_nonoverlapping(message.as_ptr(), buffer.ptr, message.len()) };

    assert_eq!(
        unsafe { ak_call_send_message(call, buffer) },
        ak_status::AK_STATUS_OK
    );
    assert_eq!(ak_call_end_send(call), ak_status::AK_STATUS_OK);
}

fn try_create_runtime(
    worker_threads: u32,
    memory_ceiling: u64,
    runtime_ctx: *mut c_void,
) -> (ak_status, ak_handle) {
    let config = ak_runtime_config {
        struct_size: std::mem::size_of::<ak_runtime_config>() as u32,
        worker_threads,
        memory_ceiling,
    };
    let mut runtime = AK_HANDLE_NONE;
    let status =
        unsafe { ak_runtime_create(&config, Some(support::on_event), runtime_ctx, &mut runtime) };
    (status, runtime)
}

fn debt_of(call: ak_handle) -> ak_call_debt {
    let mut debt = ak_call_debt::default();
    assert_eq!(
        unsafe { ak_call_debt_of(call, &mut debt) },
        ak_status::AK_STATUS_OK
    );
    debt
}

fn memory_usage(runtime: ak_handle) -> ak_memory_usage {
    let mut usage = ak_memory_usage::default();
    assert_eq!(
        unsafe { ak_runtime_memory_usage(runtime, &mut usage) },
        ak_status::AK_STATUS_OK
    );
    usage
}

fn try_start_call(channel: ak_handle, method: &str, metadata: &[u8]) -> (ak_status, ak_handle) {
    let options = ak_call_start_options {
        struct_size: std::mem::size_of::<ak_call_start_options>() as u32,
        method: ak_bytes_in {
            ptr: method.as_ptr(),
            len: method.len(),
        },
        metadata: ak_bytes_in {
            ptr: metadata.as_ptr(),
            len: metadata.len(),
        },
    };
    let mut call = AK_HANDLE_NONE;
    let status = unsafe { ak_call_start(channel, &options, std::ptr::null_mut(), &mut call) };
    (status, call)
}

fn start_call(channel: ak_handle, method: &str, metadata: &[u8]) -> ak_handle {
    let (status, call) = try_start_call(channel, method, metadata);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    call
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

#[test]
fn no_call_starts_on_a_runtime_that_is_stopping() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);

    host.stop();

    let (status, call) = try_start_call(channel, ECHO, &[]);

    assert_eq!(status, ak_status::AK_STATUS_INVALID_STATE);
    assert_eq!(call, AK_HANDLE_NONE, "nothing was started");
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
