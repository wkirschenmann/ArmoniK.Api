//! A host of the ABI, as a test needs one: one runtime at a time, a channel on it, and the
//! calls made through it.
//!
//! Beside the recorder rather than inside a test file, because every cardinality drives the
//! same ABI and there is one shape for driving it.

use std::ffi::c_void;
use std::sync::{Mutex, MutexGuard};

use armonik_transport_ffi::*;

use super::{empty_buffer, on_event, poll_until, Recorder, TestServer};

/// Held across a runtime's life: `ak_runtime_create` admits one at a time, so tests that each
/// want their own take turns rather than racing for the refusal.
static ONE_RUNTIME: Mutex<()> = Mutex::new(());

pub struct Connected {
    pub channel: ak_handle,
    pub host: Host,
    _server: TestServer,
}

impl Connected {
    pub fn with_ceiling(memory_ceiling: u64) -> Self {
        let server = TestServer::start();
        let host = Host::with_ceiling(memory_ceiling);
        let channel = host.channel(&server.endpoint);
        Self {
            channel,
            host,
            _server: server,
        }
    }

    pub fn close(&self) {
        ak_channel_release(self.channel);
        self.host.stop();
    }
}

pub struct Host {
    pub runtime: ak_handle,
    pub recorder: Box<Recorder>,
    _turn: MutexGuard<'static, ()>,
}

impl Host {
    pub fn start() -> Self {
        Self::with_ceiling(0)
    }

    pub fn with_ceiling(memory_ceiling: u64) -> Self {
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

    pub fn connected() -> Connected {
        Connected::with_ceiling(0)
    }

    pub fn channel(&self, endpoint: &str) -> ak_handle {
        self.channel_with(endpoint, "{}")
    }

    /// A channel on `endpoint`, configured by `json`, which names options and never the endpoint.
    pub fn channel_with(&self, endpoint: &str, json: &str) -> ak_handle {
        let mut channel = AK_HANDLE_NONE;
        let status = unsafe {
            ak_channel_create(
                self.runtime,
                ak_bytes_in {
                    ptr: endpoint.as_ptr(),
                    len: endpoint.len(),
                },
                ak_bytes_in {
                    ptr: json.as_ptr(),
                    len: json.len(),
                },
                &mut channel,
            )
        };
        assert_eq!(status, ak_status::AK_STATUS_OK, "{endpoint} {json}");
        channel
    }

    pub fn stop(&self) {
        assert_eq!(
            ak_runtime_begin_shutdown(self.runtime),
            ak_status::AK_STATUS_OK
        );
        self.await_state(ak_runtime_state::AK_RUNTIME_QUIESCENT);
    }

    pub fn await_state(&self, wanted: ak_runtime_state) {
        poll_until(
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
        // Nothing here may panic while a test is already unwinding. A panic during a panic
        // aborts the process, and the abort takes the failing assertion's message with it - the
        // run then says "error: test failed" and names no test, which is how a flake stays
        // unexplained however many times it is reproduced.
        //
        // So on the way out of a failure this reports rather than asserts, and does its best to
        // give the claim back so the tests after it are not all failing for a reason that is not
        // theirs either.
        let failing = std::thread::panicking();

        if ak_runtime_status(self.runtime) != ak_runtime_state::AK_RUNTIME_QUIESCENT {
            if failing {
                let _ = ak_runtime_begin_shutdown(self.runtime);
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                while std::time::Instant::now() < deadline
                    && ak_runtime_status(self.runtime) != ak_runtime_state::AK_RUNTIME_QUIESCENT
                {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            } else {
                self.stop();
            }
        }

        let destroyed = ak_runtime_destroy(self.runtime);
        if failing {
            eprintln!(
                "the fixture tore down after a failure: status {:?}, destroy {destroyed:?}, {} events held",
                ak_runtime_status(self.runtime),
                self.recorder.len()
            );
        } else {
            assert_eq!(destroyed, ak_status::AK_STATUS_OK);
        }
    }
}

pub fn lend(call: ak_handle, len: usize) -> (ak_status, ak_buffer) {
    let mut buffer = empty_buffer();
    let status = unsafe { ak_get_call_buffer(call, len, &mut buffer) };
    if status != ak_status::AK_STATUS_OK {
        assert!(buffer.owner.is_null(), "a refusal leaves *out as it was");
    }
    (status, buffer)
}

/// One message into the call, without ending the sending.
///
/// The lend is what the window refuses, so a caller sending several in a row has to wait for
/// the acquittal of each: the count is what it waits on, because the recorder keeps every
/// event and a wait for the kind alone would be satisfied by the first.
pub fn write_one(host: &Host, call: ak_handle, message: &[u8], acquitted: usize) {
    let (status, buffer) = lend(call, message.len());
    assert_eq!(status, ak_status::AK_STATUS_OK);
    assert_eq!(buffer.len, message.len());

    unsafe { std::ptr::copy_nonoverlapping(message.as_ptr(), buffer.ptr, message.len()) };

    assert_eq!(
        unsafe { ak_call_send_message(call, buffer) },
        ak_status::AK_STATUS_OK
    );
    host.recorder.await_write_dones(acquitted);
}

pub fn send_one(call: ak_handle, message: &[u8]) {
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

pub fn try_create_runtime(
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
    let status = unsafe { ak_runtime_create(&config, Some(on_event), runtime_ctx, &mut runtime) };
    (status, runtime)
}

pub fn debt_of(call: ak_handle) -> ak_call_debt {
    let mut debt = ak_call_debt::default();
    assert_eq!(
        unsafe { ak_call_debt_of(call, &mut debt) },
        ak_status::AK_STATUS_OK
    );
    debt
}

pub fn memory_usage(runtime: ak_handle) -> ak_memory_usage {
    let mut usage = ak_memory_usage::default();
    assert_eq!(
        unsafe { ak_runtime_memory_usage(runtime, &mut usage) },
        ak_status::AK_STATUS_OK
    );
    usage
}

pub fn try_start_call(channel: ak_handle, method: &str, metadata: &[u8]) -> (ak_status, ak_handle) {
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

pub fn start_call(channel: ak_handle, method: &str, metadata: &[u8]) -> ak_handle {
    let (status, call) = try_start_call(channel, method, metadata);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    call
}
