mod config;
// The readers wait for the option units of T3.2; the vocabulary they interpret does not, because
// `read_env_bool` already interprets it and there is no reason for two lists of the same
// spellings. Harvested as a module of its own because it names no option and carries no default,
// and that is the property a reviewer has to be able to check.
#[allow(dead_code, unused_macros, unused_imports)]
mod config_utils;
mod connect;
pub mod grpc;
pub mod http2;
pub mod options;
mod utils;

pub use config::{ClientConfig, ClientConfigArgs, ConfigError};
pub use connect::{connect, https_connector, ConnectionError};
#[doc(hidden)]
pub use connect::{ConfigSnafu, IoSnafu, TlsSnafu, TransportSnafu};
pub use utils::ReadEnvError;

pub mod reexports {
    pub use bytes;
    pub use http;
    pub use hyper;
    pub use hyper_rustls;
    pub use hyper_util;
    pub use rustls;
    #[cfg(feature = "serde")]
    pub use serde;
    pub use tonic;
}
