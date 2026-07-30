//! Handing the crate's `tracing` instrumentation to the host's own logging system, without ever
//! calling back into the caller's code.
//!
//! [`ak_log_init`] installs a global `tracing` subscriber that formats every event as one JSON
//! object per line — level, target, message and fields, plus the active span stack — into a bounded
//! ring buffer, and signals an event handle so the caller knows there is something to read.
//! [`ak_log_drain`] pulls lines back out. The buffer being bounded is deliberate: a slow or absent
//! reader can only ever lose log lines (tracked and reported by [`ak_log_drain`]'s `dropped` count),
//! never block or slow down the transport.
//!
//! The JSON schema is `tracing_subscriber`'s own `fmt().json()` format — timestamp, level, target,
//! `fields` (including `message`), and `spans` — which is what lets a caller hand each field to its
//! own logging system as a genuine structured property instead of interpolating them into one string.

use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use tracing_subscriber::fmt::MakeWriter;

use crate::error::ak_bytes;

struct LogState {
    /// Complete, newline-terminated JSON lines waiting to be drained, oldest first.
    lines: Mutex<VecDeque<String>>,
    /// Bytes written since the last complete line, held here because `tracing_subscriber` may call
    /// `Write::write` more than once per formatted event.
    pending: Mutex<Vec<u8>>,
    capacity: usize,
    dropped: AtomicU64,
    event: LogEventHandle,
}

#[derive(Clone, Copy)]
struct LogEventHandle(*mut std::ffi::c_void);
// SAFETY: same reasoning as `call::EventHandle`: an opaque Win32 handle with no thread affinity,
// safe to signal from any thread for its whole lifetime.
unsafe impl Send for LogEventHandle {}
// SAFETY: as above.
unsafe impl Sync for LogEventHandle {}

impl LogEventHandle {
    fn signal(self) {
        if self.0.is_null() {
            return;
        }
        #[cfg(windows)]
        {
            #[link(name = "kernel32")]
            extern "system" {
                fn SetEvent(hEvent: *mut std::ffi::c_void) -> i32;
            }
            // SAFETY: caller-owned live handle; safe to signal from any thread.
            unsafe {
                SetEvent(self.0);
            }
        }
        #[cfg(not(windows))]
        {
            let _ = self.0;
        }
    }
}

impl LogState {
    fn push_line(&self, line: String) {
        let mut lines = self
            .lines
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if lines.len() >= self.capacity {
            lines.pop_front();
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        lines.push_back(line);
        drop(lines);
        self.event.signal();
    }
}

static STATE: OnceLock<&'static LogState> = OnceLock::new();

/// A [`std::io::Write`] that accumulates bytes and splits them into complete lines on `\n`,
/// pushing each complete line into the shared ring buffer.
struct LogWriter(&'static LogState);

impl io::Write for LogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut pending = self
            .0
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pending.extend_from_slice(buf);

        while let Some(newline) = pending.iter().position(|&byte| byte == b'\n') {
            let line = pending.drain(..=newline).collect::<Vec<u8>>();
            let line = String::from_utf8_lossy(&line[..line.len() - 1]).into_owned();
            drop(pending);
            self.0.push_line(line);
            pending = self
                .0
                .pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }

        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct SharedMakeWriter(&'static LogState);

impl<'a> MakeWriter<'a> for SharedMakeWriter {
    type Writer = LogWriter;

    fn make_writer(&'a self) -> Self::Writer {
        LogWriter(self.0)
    }
}

fn level_filter_from_i32(value: i32) -> tracing_subscriber::filter::LevelFilter {
    use tracing_subscriber::filter::LevelFilter;
    match value {
        0 => LevelFilter::TRACE,
        1 => LevelFilter::DEBUG,
        2 => LevelFilter::INFO,
        3 => LevelFilter::WARN,
        4 => LevelFilter::ERROR,
        _ => LevelFilter::OFF,
    }
}

/// Install the logging bridge.
///
/// `max_level` follows this crate's level convention: `0` trace, `1` debug, `2` info, `3` warn, `4`
/// error; anything else disables logging entirely. `capacity` is the maximum number of buffered
/// lines; once full, the oldest line is dropped to make room for the newest, and the drop is
/// counted (see [`ak_log_drain`]). `event_handle`, if non-null, is signalled with `SetEvent`
/// whenever a new line is available; the caller retains ownership of it.
///
/// Returns [`crate::status::INVALID_STATE`] if called more than once (`tracing` only ever has one
/// global subscriber for the whole process) or if some other component already installed one.
///
/// # Safety
///
/// `event_handle`, if non-null, must remain a valid, open handle until the process exits — there is
/// no corresponding "close the logging bridge" call, matching how a native library loaded into a
/// host process is expected to behave.
#[no_mangle]
pub unsafe extern "C" fn ak_log_init(
    event_handle: *mut std::ffi::c_void,
    max_level: i32,
    capacity: usize,
) -> i32 {
    crate::guard::catch_unwind_status_only(|| {
        if STATE.get().is_some() {
            return crate::status::INVALID_STATE;
        }

        let state: &'static LogState = Box::leak(Box::new(LogState {
            lines: Mutex::new(VecDeque::new()),
            pending: Mutex::new(Vec::new()),
            capacity: capacity.max(1),
            dropped: AtomicU64::new(0),
            event: LogEventHandle(event_handle),
        }));

        if STATE.set(state).is_err() {
            return crate::status::INVALID_STATE;
        }

        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_max_level(level_filter_from_i32(max_level))
            .with_writer(SharedMakeWriter(state))
            .finish();

        match tracing::subscriber::set_global_default(subscriber) {
            Ok(()) => crate::status::OK,
            Err(_) => crate::status::INVALID_STATE,
        }
    })
}

/// Drain up to `max` buffered log lines into `out_lines` (an array of at least `max` entries the
/// caller allocates), oldest first. Each written entry is an owned [`ak_bytes`] to be released with
/// [`crate::error::ak_bytes_free`]; entries beyond `*out_count` are left untouched.
///
/// `*out_dropped` receives how many lines were dropped for capacity since the previous drain, then
/// resets to zero — so each call reports drops since it was last called, not a running total.
///
/// Returns [`crate::status::INVALID_STATE`] if [`ak_log_init`] was never called.
///
/// # Safety
///
/// `out_lines` must be valid for `max` writable `ak_bytes` slots. `out_count` and `out_dropped` must
/// be non-null and writable.
#[no_mangle]
pub unsafe extern "C" fn ak_log_drain(
    out_lines: *mut ak_bytes,
    max: usize,
    out_count: *mut usize,
    out_dropped: *mut u64,
) -> i32 {
    crate::guard::catch_unwind_status_only(|| {
        if out_count.is_null() || out_dropped.is_null() {
            return crate::status::NULL_ARGUMENT;
        }
        if max > 0 && out_lines.is_null() {
            return crate::status::NULL_ARGUMENT;
        }

        let Some(state) = STATE.get() else {
            return crate::status::INVALID_STATE;
        };

        let mut lines = state
            .lines
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let count = max.min(lines.len());
        for i in 0..count {
            let line = lines.pop_front().expect("count is bounded by lines.len()");
            // SAFETY: `out_lines` is valid for `max` slots per this function's contract, and `i <
            // count <= max`.
            unsafe { *out_lines.add(i) = ak_bytes::from_bytes(line) };
        }
        drop(lines);

        let dropped = state.dropped.swap(0, Ordering::Relaxed);

        // SAFETY: checked non-null above.
        unsafe {
            *out_count = count;
            *out_dropped = dropped;
        }
        crate::status::OK
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ak_log_init`/the global `tracing` subscriber can only ever be installed once per process, so
    /// every test that needs it shares one instance rather than each racing to install their own.
    fn ensure_initialized() -> (*mut std::ffi::c_void, i32) {
        static ONCE: std::sync::Once = std::sync::Once::new();
        static RESULT: Mutex<i32> = Mutex::new(0);
        ONCE.call_once(|| {
            let status = unsafe { ak_log_init(std::ptr::null_mut(), 0, 64) };
            *RESULT.lock().unwrap() = status;
        });
        (std::ptr::null_mut(), *RESULT.lock().unwrap())
    }

    #[test]
    fn a_second_init_is_reported_rather_than_replacing_the_first() {
        let (_, first) = ensure_initialized();
        assert_eq!(first, crate::status::OK);

        let second = unsafe { ak_log_init(std::ptr::null_mut(), 0, 64) };
        assert_eq!(second, crate::status::INVALID_STATE);
    }

    #[test]
    fn events_logged_through_tracing_are_drained_as_json_lines() {
        ensure_initialized();

        tracing::info!(answer = 42, "a structured event from the log bridge test");

        // The writer may still be mid-flush from another test's event; poll briefly rather than
        // asserting on the very first drain.
        let mut found = false;
        for _ in 0..50 {
            let mut buffer = vec![ak_bytes::EMPTY; 32];
            let mut count = 0usize;
            let mut dropped = 0u64;
            let status = unsafe {
                ak_log_drain(
                    buffer.as_mut_ptr(),
                    buffer.len(),
                    std::ptr::addr_of_mut!(count),
                    std::ptr::addr_of_mut!(dropped),
                )
            };
            assert_eq!(status, crate::status::OK);

            for entry in buffer.iter_mut().take(count) {
                // SAFETY: just produced by `ak_log_drain` above.
                let line = unsafe { std::slice::from_raw_parts(entry.ptr, entry.len) };
                let text = String::from_utf8_lossy(line);
                if text.contains("a structured event from the log bridge test") {
                    assert!(
                        text.contains("\"answer\":42"),
                        "fields should be structured: {text}"
                    );
                    assert!(
                        text.contains("\"level\":\"INFO\""),
                        "unexpected line: {text}"
                    );
                    found = true;
                }
                unsafe { crate::error::ak_bytes_free(*entry) };
            }
            if found {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        assert!(found, "the logged event should eventually be drained");
    }

    // "drain before init" is exercised in `tests/log_uninitialized.rs` instead of here: `STATE` is
    // a process-wide `OnceLock`, and every `#[test]` in this module runs in the same process, so
    // once any of them calls `ak_log_init` the "not yet initialized" state can never be observed
    // again in this binary. A dedicated integration test file gets its own process.
}
