//! What happens when nobody drains fast enough.
//!
//! The buffer is bounded on purpose: a log sink that is slow, blocked or misconfigured must cost log
//! lines and nothing else — never a stalled RPC. So the contract is "drop the oldest, count it, say
//! so", and this is where that is checked.
//!
//! Its own test binary because the capacity is fixed at install time and there is no uninstall: a
//! four-line buffer is the whole point here and would ruin every other test in the process.

mod common;

use common::logs;
use serial_test::serial;

/// Small enough that a handful of events overflows it several times over.
const CAPACITY: usize = 4;

fn init() {
    logs::init(logs::TRACE, CAPACITY);
}

#[test]
#[serial]
fn overflowing_drops_the_oldest_lines_and_counts_them() {
    init();
    logs::drain_all();

    const EMITTED: usize = 40;
    for index in 0..EMITTED {
        tracing::info!(index, "overflow");
    }

    let (lines, dropped) = logs::drain_all();

    assert!(
        lines.len() <= CAPACITY,
        "the buffer must not grow past its capacity: {} lines",
        lines.len()
    );
    assert_eq!(
        lines.len() as u64 + dropped,
        EMITTED as u64,
        "every line is either delivered or counted as dropped, never neither"
    );

    // The oldest go first, so what survives is the tail — the most recent events, which are the ones
    // worth having when something has just gone wrong.
    let last = lines
        .last()
        .expect("at least one line should have survived");
    assert_eq!(
        last.fields()["index"],
        serde_json::json!(EMITTED - 1),
        "the newest line should be the one kept: {}",
        last.text
    );
}

#[test]
#[serial]
fn the_dropped_count_is_since_the_last_drain_rather_than_a_running_total() {
    // A caller surfaces this as a warning, so a running total would re-warn about the same losses at
    // every drain for the rest of the process's life.
    init();
    logs::drain_all();

    for index in 0..20 {
        tracing::info!(index, "first burst");
    }
    let (_, first) = logs::drain_all();
    assert!(first > 0, "20 lines into a 4-line buffer must drop some");

    let (_, second) = logs::drain_all();
    assert_eq!(second, 0, "nothing was dropped between the two drains");

    for index in 0..20 {
        tracing::info!(index, "second burst");
    }
    let (_, third) = logs::drain_all();
    assert!(third > 0, "and a new burst is reported again");
}
