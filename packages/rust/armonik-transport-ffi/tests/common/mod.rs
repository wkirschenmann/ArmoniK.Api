//! Shared fixtures for the integration tests that drive the C ABI end to end.
//!
//! Two halves, deliberately separate:
//!
//! - [`server`] is a gRPC service to call, configurable enough to produce every outcome the ABI has
//!   to report — several responses, an error status, a hang, a failure that only stops after N
//!   attempts, a request stream it refuses to read.
//! - [`abi`] is a thin, safe wrapper over the `ak_*` entry points, so a test reads as a sequence of
//!   RPC steps rather than a wall of `unsafe`. It is also the reference for how a caller is meant to
//!   use the ABI: the same poll-then-wait loop, the same "free every `ak_bytes`" rule.
//! - [`logs`] does the same for the log bridge, and parses what it drains, so a test can assert on the
//!   structure a caller will read rather than on a substring of it.
//!
//! All are `pub(crate)` fixtures, not part of any published surface.

// Each test binary uses the parts it needs.
#![allow(dead_code)]

pub(crate) mod abi;
pub(crate) mod logs;
pub(crate) mod server;
