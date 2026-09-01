use std::sync::OnceLock;

use crate::call::CallState;
use crate::registry::Registry;
use crate::runtime::{AkChannel, AkRuntime};

pub(crate) fn runtimes() -> &'static Registry<AkRuntime> {
    static TABLE: OnceLock<Registry<AkRuntime>> = OnceLock::new();
    TABLE.get_or_init(Registry::default)
}

pub(crate) fn channels() -> &'static Registry<AkChannel> {
    static TABLE: OnceLock<Registry<AkChannel>> = OnceLock::new();
    TABLE.get_or_init(Registry::default)
}

pub(crate) fn calls() -> &'static Registry<CallState> {
    static TABLE: OnceLock<Registry<CallState>> = OnceLock::new();
    TABLE.get_or_init(Registry::default)
}
