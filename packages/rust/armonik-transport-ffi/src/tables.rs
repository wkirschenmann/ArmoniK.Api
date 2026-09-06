use std::sync::OnceLock;

use crate::call::CallState;
use crate::channel::AkChannel;
use crate::registry::{Registry, CALLS, CHANNELS, RUNTIMES};
use crate::runtime::AkRuntime;

pub(crate) fn runtimes() -> &'static Registry<AkRuntime> {
    static TABLE: OnceLock<Registry<AkRuntime>> = OnceLock::new();
    TABLE.get_or_init(|| Registry::new(RUNTIMES))
}

pub(crate) fn channels() -> &'static Registry<AkChannel> {
    static TABLE: OnceLock<Registry<AkChannel>> = OnceLock::new();
    TABLE.get_or_init(|| Registry::new(CHANNELS))
}

pub(crate) fn calls() -> &'static Registry<CallState> {
    static TABLE: OnceLock<Registry<CallState>> = OnceLock::new();
    TABLE.get_or_init(|| Registry::new(CALLS))
}
