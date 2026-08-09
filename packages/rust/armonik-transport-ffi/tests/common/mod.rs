//! Fixtures shared by the tests that drive this library end to end, and by the example that serves
//! the same thing to a host application.
//!
//! Two halves, deliberately separate:
//!
//! - [`server`] is a gRPC service to point a request at, configurable enough to produce every
//!   outcome the ABI has to report: several responses, an injected status, a hang, and a hang that
//!   never reads the request either.
//! - [`abi`] is a thin, safe wrapper over the `ak_*` entry points, so a test reads as a sequence of
//!   request steps rather than a wall of `unsafe`. It is also the reference for how a caller is
//!   meant to use the ABI: one armed operation at a time, everything copied out of a borrowed
//!   payload before the callback returns.

// Each test binary, and the example, uses the parts it needs.
#![allow(dead_code)]

pub(crate) mod abi;
pub(crate) mod server;
