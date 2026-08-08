//! The single tokio runtime every client and call in this crate runs on.
//!
//! One process-wide multi-threaded runtime is created lazily on first use and lives for the life of
//! the process; there is no API to shut it down, matching how a native library loaded into a host
//! process is expected to behave.

use std::sync::OnceLock;

use tokio::runtime::Runtime;

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// Get the shared runtime, creating it on first use.
///
/// # Panics
///
/// Panics if the runtime cannot be created (e.g. the OS refuses to spawn worker threads). This is
/// caught like any other panic by the `catch_unwind_*` wrappers at the ABI boundary.
pub(crate) fn handle() -> &'static Runtime {
    RUNTIME.get_or_init(|| Runtime::new().expect("failed to create the ArmoniK FFI tokio runtime"))
}

/// How many tasks are currently alive on the shared runtime.
///
/// Not part of the C ABI - no `extern "C"`, no `#[no_mangle]`, so it never reaches the generated
/// header - and not part of anything a consumer is meant to call. It exists because the leak
/// assertions in `tests/request_lifecycle.rs` have nothing else to look at: a cancelled or abandoned
/// call that left its driving task parked forever is invisible from the outside, and "the tests
/// passed" is not evidence that the runtime came back to rest.
#[doc(hidden)]
pub fn alive_tasks() -> usize {
    handle().metrics().num_alive_tasks()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_runtime_is_created_once_and_reused() {
        let first = std::ptr::from_ref(handle());
        let second = std::ptr::from_ref(handle());
        assert_eq!(first, second);
    }

    #[test]
    fn the_runtime_can_actually_run_futures() {
        let value = handle().block_on(async { 1 + 1 });
        assert_eq!(value, 2);
    }
}
