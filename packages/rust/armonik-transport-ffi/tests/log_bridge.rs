//! The structured-logging bridge, as the .NET side will read it.
//!
//! The point of this bridge is that a Rust field arrives at `ILogger` as a real structured property —
//! something Serilog or Seq can index — rather than interpolated into a sentence. That only works if
//! the JSON keeps its shape and its types, so these tests parse what they drain and assert on the
//! structure, not on a substring of it.
//!
//! Its own test binary, because the bridge is process-wide: one `tracing` subscriber and one ring
//! buffer, installed once and never removed. `#[serial]`, for the same reason — two tests draining at
//! once would each take some of the other's lines.

mod common;

use armonik_transport_ffi::status;
use bytes::Bytes;
use common::abi::{Client, Kind, StartOptions};
use common::logs;
use common::server::{serve, TestService, METHOD_PATH};
use serial_test::serial;

/// Room for whatever `hyper`/`tonic` say at debug level around a real call, so nothing under test is
/// dropped for capacity. Overflow gets its own binary, where a tiny capacity is the point.
const CAPACITY: usize = 1024;

fn init() {
    logs::init(logs::DEBUG, CAPACITY);
}

#[test]
#[serial]
fn an_event_carries_its_level_target_and_fields_separately() {
    init();
    logs::drain_all();

    tracing::info!(
        answer = 42,
        session = "session-id",
        "the bridge carried this"
    );

    let line = logs::wait_for("the event just logged", |line| {
        line.message() == "the bridge carried this"
    });

    assert_eq!(line.level(), "INFO");
    assert_eq!(
        line.target(),
        "log_bridge",
        "the target becomes the .NET logger category, so it must be the emitting module"
    );
    assert!(
        line.json["timestamp"].is_string(),
        "a log line needs a timestamp: {}",
        line.text
    );

    // Typed, not stringified: `42` has to stay a number, or every numeric property arrives at the
    // .NET sink as text and stops being aggregatable.
    assert_eq!(line.fields()["answer"], serde_json::json!(42));
    assert_eq!(line.fields()["session"], serde_json::json!("session-id"));
}

#[test]
#[serial]
fn the_active_span_stack_travels_with_the_event() {
    // `RustLogBridge` flattens these into `span.*` properties, outermost first, so both the order and
    // the per-span fields have to survive. This is what carries the context — which endpoint, which
    // session — onto an event that never mentions it.
    init();
    logs::drain_all();

    let outer = tracing::info_span!("Client", endpoint = "https://localhost:5003");
    let _outer = outer.enter();
    let inner = tracing::info_span!("Sessions::list", page = 3);
    let _inner = inner.enter();

    tracing::info!("an event inside two spans");

    let line = logs::wait_for("the event inside two spans", |line| {
        line.message() == "an event inside two spans"
    });

    let spans = line.json["spans"]
        .as_array()
        .unwrap_or_else(|| panic!("`spans` should be an array: {}", line.text));
    assert_eq!(
        spans.len(),
        2,
        "both spans should be reported: {}",
        line.text
    );
    assert_eq!(spans[0]["name"], serde_json::json!("Client"));
    assert_eq!(
        spans[0]["endpoint"],
        serde_json::json!("https://localhost:5003")
    );
    assert_eq!(spans[1]["name"], serde_json::json!("Sessions::list"));
    assert_eq!(spans[1]["page"], serde_json::json!(3));
}

#[test]
#[serial]
fn a_retry_inside_a_real_call_is_reported_through_the_bridge() {
    // End to end, and the reason this bridge exists: something that happens deep inside the native
    // transport, invisible to the caller, has to reach the .NET log with its fields intact. A retry is
    // exactly that — the call succeeds, and without this line nobody ever knows it took three tries.
    init();
    logs::drain_all();

    let service = TestService::canned([Bytes::from_static(b"finally")])
        .failing_first(1, armonik_transport::reexports::tonic::Code::Unavailable);
    let endpoint = serve(service.clone());

    let client = Client::to(&endpoint, &Client::quick_retry());
    let call = client.start(METHOD_PATH, Kind::Unary, StartOptions::default());
    call.send_ok(b"ping");
    call.close_send_ok();
    let (_, outcome) = call.drain();
    assert_eq!(outcome.code, 0);
    assert_eq!(service.attempts(), 2);

    let line = logs::wait_for("the retry the transport performed", |line| {
        line.message() == "Retrying a call from the FFI layer"
    });

    assert_eq!(line.level(), "DEBUG");
    assert_eq!(line.target(), "armonik_transport_ffi::call");
    assert_eq!(
        line.fields()["attempts_made"],
        serde_json::json!(1),
        "the first attempt is the one that failed: {}",
        line.text
    );
    assert_eq!(
        line.fields()["code"],
        serde_json::json!("Unavailable"),
        "the status that triggered the retry should be named: {}",
        line.text
    );
    assert!(
        line.fields()["delay"].is_string(),
        "the backoff should be reported: {}",
        line.text
    );
}

#[test]
#[serial]
fn a_drain_is_bounded_by_max_and_leaves_the_rest_buffered() {
    // .NET drains into a fixed-size array, so a bound that was not respected would be a buffer
    // overrun, and lines beyond it must wait rather than be discarded.
    init();
    logs::drain_all();

    for index in 0..6 {
        tracing::info!(index, "bounded drain");
    }

    // Wait until all six have made it through the writer before bounding anything, otherwise this
    // would be asserting on how fast `tracing` flushes.
    logs::wait_for("the last of the six lines", |line| {
        line.message() == "bounded drain" && line.fields()["index"] == serde_json::json!(5)
    });

    for index in 0..4 {
        tracing::info!(index, "second batch");
    }
    logs::wait_for("the last of the second batch", |line| {
        line.message() == "second batch" && line.fields()["index"] == serde_json::json!(3)
    });

    for index in 0..4 {
        tracing::info!(index, "third batch");
    }
    std::thread::sleep(std::time::Duration::from_millis(50));

    let (first, dropped) = logs::drain(2);
    assert_eq!(first.len(), 2, "exactly the two slots that were offered");
    assert_eq!(dropped, 0, "nothing was dropped for capacity here");

    let (rest, _) = logs::drain_all();
    assert!(
        !rest.is_empty(),
        "the lines beyond the bound should still be buffered"
    );
    assert!(
        rest.iter().any(|line| line.message() == "third batch"),
        "and they should be the ones that did not fit"
    );
}

#[test]
#[serial]
fn draining_rejects_a_null_out_parameter_rather_than_writing_through_it() {
    init();

    let mut count = 0usize;
    let mut dropped = 0u64;

    // SAFETY: every call below deliberately passes a null the ABI documents as rejected.
    unsafe {
        assert_eq!(
            armonik_transport_ffi::ak_log_drain(
                std::ptr::null_mut(),
                4,
                std::ptr::addr_of_mut!(count),
                std::ptr::addr_of_mut!(dropped),
            ),
            status::NULL_ARGUMENT,
            "asking for four lines with nowhere to put them"
        );
        assert_eq!(
            armonik_transport_ffi::ak_log_drain(
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::addr_of_mut!(dropped),
            ),
            status::NULL_ARGUMENT
        );
        assert_eq!(
            armonik_transport_ffi::ak_log_drain(
                std::ptr::null_mut(),
                0,
                std::ptr::addr_of_mut!(count),
                std::ptr::null_mut(),
            ),
            status::NULL_ARGUMENT
        );
    }
}

#[test]
#[serial]
fn a_second_init_is_reported_rather_than_replacing_the_first() {
    init();

    // SAFETY: a null handle is documented as "no wake-up"; the second install is the assertion.
    let again = unsafe { armonik_transport_ffi::ak_log_init(std::ptr::null_mut(), logs::DEBUG, 8) };
    assert_eq!(
        again,
        status::INVALID_STATE,
        "`tracing` has one global subscriber per process, so this cannot be silently ignored"
    );
}
