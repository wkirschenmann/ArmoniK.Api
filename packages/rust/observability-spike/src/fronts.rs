//! Point 2: the alternatives to `obs::RtSub`, built on `tracing-subscriber`'s own filters, so
//! that the cost of a disabled event can be measured against them.

use std::ffi::c_void;
use std::sync::Arc;

use tracing::{Dispatch, Event, Subscriber};
use tracing_subscriber::filter::{EnvFilter, Targets};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::registry::{LookupSpan, Registry};
use tracing_subscriber::reload;

use crate::record::{with_record, LogCallback};

/// Renders every event it is given and calls the callback: no filtering of its own.
pub struct SinkLayer {
    pub callback: LogCallback,
    pub ctx: usize,
}

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for SinkLayer {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        with_record(event, |record| unsafe {
            (self.callback)(self.ctx as *mut c_void, record)
        });
    }
}

/// A filter in a `Dispatch` that can be replaced: the handle swaps it and rebuilds the interest
/// cache.
pub enum Handle {
    Env(reload::Handle<EnvFilter, Registry>),
    Targets(reload::Handle<Targets, Registry>),
    EnvPerLayer(reload::Handle<EnvFilter, Registry>),
}

impl Handle {
    pub fn set(&self, directives: &str) {
        match self {
            Handle::Env(handle) | Handle::EnvPerLayer(handle) => handle
                .reload(EnvFilter::try_new(directives).expect("directives"))
                .expect("reloaded"),
            Handle::Targets(handle) => handle
                .reload(directives.parse::<Targets>().expect("directives"))
                .expect("reloaded"),
        }
    }
}

/// `EnvFilter` as a global filter layer under the sink, behind `reload`.
pub fn env_global(directives: &str, callback: LogCallback, ctx: *mut c_void) -> (Dispatch, Handle) {
    let (filter, handle) = reload::Layer::new(EnvFilter::try_new(directives).expect("directives"));
    let subscriber = Registry::default()
        .with(filter)
        .with(SinkLayer {
            callback,
            ctx: ctx as usize,
        });
    (Dispatch::new(subscriber), Handle::Env(handle))
}

/// `Targets` as a global filter layer under the sink, behind `reload`.
pub fn targets_global(directives: &str, callback: LogCallback, ctx: *mut c_void) -> (Dispatch, Handle) {
    let (filter, handle) = reload::Layer::new(directives.parse::<Targets>().expect("directives"));
    let subscriber = Registry::default()
        .with(filter)
        .with(SinkLayer {
            callback,
            ctx: ctx as usize,
        });
    (Dispatch::new(subscriber), Handle::Targets(handle))
}

/// `EnvFilter` as the sink layer's own per-layer filter, behind `reload`.
pub fn env_per_layer(directives: &str, callback: LogCallback, ctx: *mut c_void) -> (Dispatch, Handle) {
    use tracing_subscriber::Layer as _;
    let (filter, handle) = reload::Layer::new(EnvFilter::try_new(directives).expect("directives"));
    let subscriber = Registry::default().with(
        SinkLayer {
            callback,
            ctx: ctx as usize,
        }
        .with_filter(filter),
    );
    (Dispatch::new(subscriber), Handle::EnvPerLayer(handle))
}

/// Keeps `Arc` in the public surface for callers that share a handle.
pub type Shared<T> = Arc<T>;
