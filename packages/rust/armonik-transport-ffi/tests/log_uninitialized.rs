//! Draining before `ak_log_init` was ever called.
//!
//! Kept in its own file so it gets its own process: `armonik_transport_ffi`'s logging state lives
//! in a process-wide `OnceLock`, so once anything in a process calls `ak_log_init`, "never
//! initialized" can no longer be observed there.

#[test]
fn draining_before_init_is_reported_rather_than_reading_nothing() {
    let mut count = 0usize;
    let mut dropped = 0u64;

    let status = unsafe {
        armonik_transport_ffi::ak_log_drain(
            std::ptr::null_mut(),
            0,
            std::ptr::addr_of_mut!(count),
            std::ptr::addr_of_mut!(dropped),
        )
    };

    assert_eq!(status, armonik_transport_ffi::status::INVALID_STATE);
}
