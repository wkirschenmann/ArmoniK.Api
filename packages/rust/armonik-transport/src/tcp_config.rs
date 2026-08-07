//! TCP-level socket options.
//!
//! A unit of fields, not a naming scheme: the embedding composes them with `#[serde(flatten)]`
//! under a prefix of its own, so the same unit serves however many embeddings read TCP options,
//! and grouping these fields changes no environment variable a deployment already sets.

use std::time::Duration;

/// TCP-level socket options.
///
/// Each field names one option; the full name a source spells is the embedding's prefix followed
/// by that name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "PascalCase", default))]
#[non_exhaustive]
pub struct TcpConfig {
    /// TCP keepalive duration (e.g. `30s`), defaults to no keepalive. `Keepalive`.
    #[cfg_attr(feature = "serde", serde(deserialize_with = "keepalive"))]
    pub keepalive: Option<Duration>,
    /// Interval between TCP keepalive probes (e.g. `5s`), defaults to OS default.
    /// `KeepaliveInterval`.
    #[cfg_attr(feature = "serde", serde(deserialize_with = "keepalive_interval"))]
    pub keepalive_interval: Option<Duration>,
    /// Number of TCP keepalive retries, defaults to OS default. `KeepaliveRetries`.
    #[cfg_attr(feature = "serde", serde(deserialize_with = "keepalive_retries"))]
    pub keepalive_retries: Option<u32>,
    /// Enable Nagle's algorithm (disable TCP_NODELAY), defaults to false. `NagleAlgorithm`. See
    /// [`crate::TlsConfig::allow_unsafe_connection`] for the accepted spellings.
    #[cfg_attr(feature = "serde", serde(deserialize_with = "nagle_algorithm"))]
    pub nagle_algorithm: bool,
}

/// [`crate::config::bool_option`], naming this field's own option: a flattened `serde` source
/// buffers values before handing them over, so the name is no longer available by the time the
/// value is read.
#[cfg(feature = "serde")]
fn nagle_algorithm<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    crate::config::bool_option("TcpNagleAlgorithm", deserializer)
}

/// [`crate::config::optional_duration`], naming this field's own option.
#[cfg(feature = "serde")]
fn keepalive<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Duration>, D::Error> {
    crate::config::optional_duration("TcpKeepalive", deserializer)
}

/// [`crate::config::optional_duration`], naming this field's own option.
#[cfg(feature = "serde")]
fn keepalive_interval<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Duration>, D::Error> {
    crate::config::optional_duration("TcpKeepaliveInterval", deserializer)
}

/// [`crate::config::optional_u32`], naming this field's own option.
#[cfg(feature = "serde")]
fn keepalive_retries<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u32>, D::Error> {
    crate::config::optional_u32("TcpKeepaliveRetries", deserializer)
}
