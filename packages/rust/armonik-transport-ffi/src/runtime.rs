//! The single tokio runtime everything in this crate runs on.
//!
//! One process-wide multi-threaded runtime, created lazily on first use and living for the life of
//! the process. There is no entry point that shuts it down, which is how a native library loaded
//! into a host process is expected to behave.

use std::sync::OnceLock;

use tokio::runtime::Runtime;

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// The shared runtime, created on first use.
///
/// # Panics
///
/// Panics if the runtime cannot be created, for instance because the OS refuses to spawn worker
/// threads. That is caught like any other panic by the guards at the ABI boundary.
pub(crate) fn handle() -> &'static Runtime {
    RUNTIME.get_or_init(|| Runtime::new().expect("failed to create the ArmoniK FFI tokio runtime"))
}

/// How many tasks are currently alive on the shared runtime.
///
/// Not part of the C ABI - nothing here is `extern "C"`, so none of it reaches the generated header -
/// and not something a consumer is meant to call. It answers the one question the outside cannot:
/// work that left a task parked forever is invisible from there, and "the tests passed" is not
/// evidence that the runtime came back to rest.
///
/// A request that leaks is caught by its context instead, which the completion gives back and the
/// completion is a task's last act. This counts what no request owns.
#[doc(hidden)]
pub fn alive_tasks() -> usize {
    handle().metrics().num_alive_tasks()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // Miri cannot drive tokio's I/O reactor: it reaches the platform's completion-port calls and
    // stops with "unsupported operation".
    #[cfg_attr(miri, ignore)]
    fn the_runtime_is_created_once_and_reused() {
        let first = std::ptr::from_ref(handle());
        let second = std::ptr::from_ref(handle());
        assert_eq!(first, second);
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn the_runtime_can_actually_run_futures() {
        let value = handle().block_on(async { 1 + 1 });
        assert_eq!(value, 2);
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn an_idle_runtime_reports_no_alive_tasks() {
        // A runtime that has never been given work has to answer zero, or nothing read from this
        // counter afterwards means anything.
        handle().block_on(async {});
        assert_eq!(alive_tasks(), 0);
    }
}
