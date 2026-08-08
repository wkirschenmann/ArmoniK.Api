//! Shared fixtures for the integration tests that drive the C ABI end to end.
//!
//! Three halves, deliberately separate:
//!
//! - [`server`] is a gRPC service to call, configurable enough to produce every outcome the ABI has
//!   to report - several responses, an error status, a hang, a failure that only stops after N
//!   attempts, a request stream it refuses to read.
//! - [`spike`] is the same machinery arranged as one service with a method per behaviour, which is
//!   what the C# harness reaches through a single channel. The `spike_server` example serves it.
//! - [`abi`] is a thin, safe wrapper over the `ak_*` entry points, so a test reads as a sequence of
//!   request steps rather than a wall of `unsafe`. It is also the reference for how a caller is
//!   meant to use the ABI: one armed operation at a time, `COMPLETED` releases everything.
//!
//! All are `pub(crate)` fixtures, not part of any published surface.

// Each test binary, and the example, uses the parts it needs.
#![allow(dead_code)]

pub(crate) mod abi;
pub(crate) mod server;
pub(crate) mod spike;
