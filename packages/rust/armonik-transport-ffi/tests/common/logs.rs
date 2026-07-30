//! Draining the log bridge from a test, and reading what came out as JSON.
//!
//! The bridge is process-wide — one `tracing` subscriber, one ring buffer — so a test binary using it
//! initialises it once and every test in that binary shares it. That is also why the tests that use
//! this are `#[serial]`: two draining at the same time would each take some of the other's lines.

use std::sync::Once;
use std::time::{Duration, Instant};

use armonik_transport_ffi::{ak_bytes, ak_bytes_free, ak_log_drain, ak_log_init, status};

/// This crate's level numbering, as the header documents it.
pub(crate) const TRACE: i32 = 0;
pub(crate) const DEBUG: i32 = 1;

/// Install the bridge, once per process.
///
/// No event handle: a test polls, where a caller waits. The wake-up itself is Windows-only and is
/// covered where it belongs, on the call handle.
pub(crate) fn init(max_level: i32, capacity: usize) {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // SAFETY: a null handle is documented as "no wake-up", and this runs exactly once.
        let started = unsafe { ak_log_init(std::ptr::null_mut(), max_level, capacity) };
        assert_eq!(started, status::OK, "the log bridge should install");
    });
}

/// One drained log line, still as text plus its parsed form.
pub(crate) struct Line {
    pub(crate) text: String,
    pub(crate) json: serde_json::Value,
}

impl Line {
    /// The event's own fields, i.e. what a caller turns into structured log properties.
    pub(crate) fn fields(&self) -> &serde_json::Value {
        &self.json["fields"]
    }

    pub(crate) fn message(&self) -> &str {
        self.fields()["message"].as_str().unwrap_or_default()
    }

    pub(crate) fn level(&self) -> &str {
        self.json["level"].as_str().unwrap_or_default()
    }

    pub(crate) fn target(&self) -> &str {
        self.json["target"].as_str().unwrap_or_default()
    }
}

/// Drain up to `max` lines, returning them and how many were dropped since the last drain.
pub(crate) fn drain(max: usize) -> (Vec<Line>, u64) {
    let mut buffer = vec![
        ak_bytes {
            ptr: std::ptr::null(),
            len: 0,
            owner: std::ptr::null_mut(),
        };
        max.max(1)
    ];
    let mut count = 0usize;
    let mut dropped = 0u64;

    // SAFETY: `buffer` has `max` writable slots and both counters are live locals.
    let code = unsafe {
        ak_log_drain(
            buffer.as_mut_ptr(),
            max,
            std::ptr::addr_of_mut!(count),
            std::ptr::addr_of_mut!(dropped),
        )
    };
    assert_eq!(code, status::OK, "draining an installed bridge should work");
    assert!(count <= max, "a drain must respect its own bound");

    let mut lines = Vec::with_capacity(count);
    for entry in buffer.iter().take(count) {
        // SAFETY: produced by `ak_log_drain` just above, readable until freed.
        let bytes = unsafe { std::slice::from_raw_parts(entry.ptr, entry.len) };
        let text = String::from_utf8(bytes.to_vec()).expect("a log line should be UTF-8");
        let json = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("a log line should be JSON ({error}): {text}"));
        lines.push(Line { text, json });
        // SAFETY: each entry is freed exactly once, here.
        unsafe { ak_bytes_free(*entry) };
    }

    (lines, dropped)
}

/// Drain everything currently buffered, in as many passes as it takes.
pub(crate) fn drain_all() -> (Vec<Line>, u64) {
    let mut all = Vec::new();
    let mut dropped = 0;
    loop {
        let (lines, more) = drain(64);
        dropped += more;
        let exhausted = lines.len() < 64;
        all.extend(lines);
        if exhausted {
            return (all, dropped);
        }
    }
}

/// Drain until a line satisfies `wanted`, or fail the test.
///
/// Polling rather than draining once: `tracing_subscriber` writes a line in as many `write` calls as
/// it likes, and the events of interest here are produced by a background task, so "not there yet" is
/// an ordinary state rather than a failure.
pub(crate) fn wait_for(what: &str, mut wanted: impl FnMut(&Line) -> bool) -> Line {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let (lines, _) = drain_all();
        if let Some(line) = lines.into_iter().find(&mut wanted) {
            return line;
        }
        assert!(Instant::now() < deadline, "no log line matched {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}
