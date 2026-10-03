//! The library's own threads, counted with the crate's test hooks.

// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use std::sync::Arc;
use std::time::Duration;

use armonik_transport_ffi::hooks;
use armonik_transport_ffi::*;
use support::host::*;
use support::TestServer;

/// A channel's thread ends once the host has released the channel, and the teardown ends the
/// thread of a channel the host still holds before it reports QUIESCENT, which promises that no
/// thread of the runtime is left.
#[test]
fn a_channel_thread_ends_with_its_channel_and_none_outlives_quiescence() {
    let server = TestServer::start();
    let host = Host::start();
    let released = host.channel(&server.endpoint);
    let kept = host.channel(&server.endpoint);
    assert_eq!(hooks::channel_threads(), 2);
    // Slower to end than the rest of the teardown, so that QUIESCENT is early unless it waits.
    hooks::channel_thread_ending(Some(Arc::new(|| {
        std::thread::sleep(Duration::from_millis(200))
    })));
    let _unhook = Unhook;

    ak_channel_release(released);
    support::poll_until(
        || hooks::channel_threads() == 1,
        || format!("{} channel threads left", hooks::channel_threads()),
    );

    host.stop();
    assert_eq!(hooks::channel_threads(), 0);
    ak_channel_release(kept);
}

struct Unhook;

impl Drop for Unhook {
    fn drop(&mut self) {
        hooks::channel_thread_ending(None);
    }
}
