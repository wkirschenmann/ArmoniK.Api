//! The options a caller sets on a channel, as a document carries them.
//!
//! Structured and typed, because the schema derived from these types is what generates the
//! options class a .NET caller fills in: a number is a number, a group of options is an object,
//! and every constraint that can be said here is said here rather than only in the code that
//! enforces it. What a type cannot say - that an endpoint names a scheme this engine speaks -
//! the transport says, by option name.

use std::time::Duration;

/// The largest window either side of a call may be given.
///
/// A window becomes a `tokio` semaphore, which refuses more than `Semaphore::MAX_PERMITS`
/// permits, and that limit is `usize::MAX >> 3` - so it is far larger on a 64-bit target than on
/// a 32-bit one. The schema is one file for every target, so the bound it states is the tighter
/// of the two; `a_window_the_schema_admits_is_one_a_semaphore_admits` is what keeps that true.
pub const LARGEST_WINDOW: i32 = 536_870_910;

/// A duration, in seconds.
///
/// Seconds rather than a `Duration`, whose schema is `{ secs, nanos }` - this crate's memory
/// layout rather than anything a document would write. One unit, no suffix to parse, and
/// `TimeSpan.FromSeconds` on the other side takes exactly this.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Seconds(pub f64);

impl From<Seconds> for Duration {
    fn from(value: Seconds) -> Self {
        Duration::from_secs_f64(value.0)
    }
}

/// What the transport does, beyond reaching the endpoint it was given.
///
/// The endpoint is not here: it is the one value a channel cannot be created without, so it
/// crosses the ABI as its own argument rather than as an option that happens to be mandatory.
/// Everything in this document has a default, and `{}` is a valid configuration.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(rename_all = "PascalCase", deny_unknown_fields)
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct TransportOptions {
    /// How long a dial may take before it is given up on. Absent for the engine's own default.
    ///
    /// Zero is refused: no dial could beat it, so it names a channel that can never connect.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    // `range` has no exclusive bound, and zero has to be excluded rather than admitted, so the
    // keyword is set directly.
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Seconds", extend("exclusiveMinimum" = 0.0))
    )]
    pub connect_timeout: Option<Seconds>,
}

/// What a caller may set on one channel.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(rename_all = "PascalCase", deny_unknown_fields)
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct ChannelOptions {
    /// What the transport does, beyond reaching the endpoint.
    #[cfg_attr(feature = "serde", serde(default))]
    pub transport: TransportOptions,

    /// What this client calls itself in `user-agent`. Absent for the engine's own.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "String", length(min = 1)))]
    pub user_agent: Option<String>,

    /// The largest message this client will accept, in bytes.
    ///
    /// No upper bound: the largest a caller can name is a channel that refuses nothing. Zero is
    /// the one that means something, and it means no message can ever be received.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "i32", range(min = 1)))]
    pub max_receive_message_size: Option<i32>,

    /// How many messages a call may have sent and unacquitted at once.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "i32", range(min = 1, max = LARGEST_WINDOW))
    )]
    pub max_sends_in_flight: Option<i32>,

    /// How many events the engine may hold for a call the host has not read from.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "i32", range(min = 1, max = LARGEST_WINDOW))
    )]
    pub delivery_credits: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bound the schema states has to be one every target can honour, and the tighter of the
    /// two targets is what it states - so on a 64-bit host this passes with room to spare and on
    /// a 32-bit one it passes by exactly one.
    #[test]
    fn a_window_the_schema_admits_is_one_a_semaphore_admits() {
        assert!(
            (LARGEST_WINDOW as usize) < tokio::sync::Semaphore::MAX_PERMITS,
            "the schema admits {LARGEST_WINDOW}, which a semaphore of {} would refuse",
            tokio::sync::Semaphore::MAX_PERMITS
        );
    }
}
