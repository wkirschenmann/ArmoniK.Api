//! Point 5: the structure, its versioning, and a channel's numbers surviving its close.

use observability_spike::stats::{RuntimeStats, StatsV1};
use std::sync::Arc;

#[test]
fn a_closed_channels_counters_stay_in_the_total_and_gauges_do_not() {
    let runtime = Arc::new(RuntimeStats::default());
    let a = runtime.open_channel();
    let b = runtime.open_channel();
    a.stats.dials.add(2);
    a.stats.calls_started.add(5);
    a.stats.calls_in_flight.add(3);
    b.stats.dials.add(1);
    b.stats.retries.add(4);

    let mut out = StatsV1 {
        struct_size: std::mem::size_of::<StatsV1>() as u32,
        ..Default::default()
    };
    runtime.read(&mut out);
    assert_eq!((out.channels_open, out.dials, out.calls_started, out.calls_in_flight), (2, 3, 5, 3));

    drop(a);
    runtime.read(&mut out);
    assert_eq!(out.channels_open, 1);
    assert_eq!(out.dials, 3, "a counter never goes backwards");
    assert_eq!(out.calls_started, 5);
    assert_eq!(out.calls_in_flight, 0, "a gauge is what is open now");
    assert_eq!(out.retries, 4);
}

/// A host built against an older header passes a shorter structure: it gets the prefix it knows
/// and what follows is untouched.
#[test]
fn a_shorter_structure_receives_its_prefix_only() {
    let runtime = Arc::new(RuntimeStats::default());
    let a = runtime.open_channel();
    a.stats.calls_started.add(9);
    a.stats.goaways.add(7);

    // Room for the header and four counters: struct_size, version, channels_open, calls_started,
    // calls_in_flight, calls_completed.
    let known = 8 + 8 * 4;
    let mut buffer = [0xAAu8; std::mem::size_of::<StatsV1>()];
    let out = buffer.as_mut_ptr() as *mut StatsV1;
    unsafe { (*out).struct_size = known as u32 };
    runtime.read(unsafe { &mut *out });

    let read = unsafe { *out };
    assert_eq!(read.struct_size as usize, known, "what was written");
    assert_eq!(read.version, 1);
    assert_eq!(read.channels_open, 1);
    assert_eq!(read.calls_started, 9);
    assert!(buffer[known..].iter().all(|b| *b == 0xAA), "nothing past the caller's size");
}
