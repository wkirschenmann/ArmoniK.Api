//! Fixtures shared by the tests that drive this library end to end, and by the example that serves
//! the same thing to a host application.
//!
//! [`server`] is a gRPC service to point a request at, configurable enough to produce every outcome
//! the ABI has to report: several responses, an injected status, a hang, and a hang that never reads
//! the request either.

// Each test binary, and the example, uses the parts it needs.
#![allow(dead_code)]

pub(crate) mod server;
