//! Points 1 to 3: one runtime's logging state - its filter, its callback, the events kept for it -
//! and the subscribers that front it.
//!
//! Two fronts over the same state, so that they can be measured against each other:
//! `RtSub`, a bare `Subscriber` that carries events only, and `RtLayer`, a `Layer` over a
//! `Registry`, which also carries spans.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use arc_swap::ArcSwap;
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::Interest;
use tracing::{Dispatch, Event, Level, Metadata, Subscriber};
use tracing_core::LevelFilter;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::registry::{LookupSpan, Registry};

use crate::fast_filter::{FastFilter, FilterCell};
use crate::record::{level_of, with_record, LogCallback, OwnedRecord};
use crate::trace::TraceState;

/// The engine's own default: itself at info, everything else at warn, which keeps h2, hyper and
/// tonic quiet unless a directive brings them back.
pub const DEFAULT_FILTER: &str = "warn,armonik_transport=info,armonik_transport_ffi=info";

#[derive(Debug)]
pub struct FilterRefused(pub String);

/// `Targets` is lenient: it reads a bare word as a target at trace level, so that `inof` turns
/// every level off, and it accepts `h2[span]=debug` without honouring it. A directive is checked
/// first, so that what is accepted is only `level` and `target=level`.
pub fn parse_filter(directives: &str) -> Result<Targets, FilterRefused> {
    for part in directives.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err(FilterRefused("an empty directive".to_owned()));
        }
        let (target, level) = match part.split_once('=') {
            Some((target, level)) => (Some(target.trim()), level.trim()),
            None => (None, part),
        };
        level
            .parse::<LevelFilter>()
            .map_err(|_| FilterRefused(format!("`{part}`: `{level}` is not a level")))?;
        if let Some(target) = target {
            let plain = !target.is_empty()
                && target
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':');
            if !plain {
                return Err(FilterRefused(format!("`{part}`: `{target}` is not a target")));
            }
        }
    }
    directives
        .parse::<Targets>()
        .map_err(|error| FilterRefused(error.to_string()))
}

fn max_level_of(targets: &Targets) -> LevelFilter {
    let mut max = targets.default_level().unwrap_or(LevelFilter::OFF);
    for (_, level) in targets.iter() {
        if level > max {
            max = level;
        }
    }
    max
}

/// What the runtime's dispatcher does with an event.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// The configuration is loading: everything up to debug is kept, since the filter that will
    /// select it is among what is loading.
    Capture = 0,
    /// No callback: nothing is logged.
    Off = 1,
    /// A callback is registered and the kept events are being delivered; new ones queue behind.
    Drain = 2,
    Live = 3,
}

struct Registered {
    callback: LogCallback,
    ctx: usize,
}

pub struct RtObs {
    pub id: u64,
    mode: AtomicU8,
    filter: FilterCell,
    max_level: AtomicU8,
    kept: Mutex<Vec<OwnedRecord>>,
    registered: ArcSwap<Option<Registered>>,
    in_callback: AtomicUsize,
    pub trace: TraceState,
}

thread_local! {
    static IN_LOG_CALLBACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The filter would not parse; the one in force stays.
    InvalidFilter,
    /// Asked from inside the log callback, which it would wait for.
    FromInsideCallback,
}

fn level_byte(level: LevelFilter) -> u8 {
    // OFF is the smallest, TRACE the largest, in `LevelFilter`'s own ordering inverted: store
    // the number of levels enabled.
    match level {
        LevelFilter::OFF => 0,
        LevelFilter::ERROR => 1,
        LevelFilter::WARN => 2,
        LevelFilter::INFO => 3,
        LevelFilter::DEBUG => 4,
        _ => 5,
    }
}

fn level_from_byte(byte: u8) -> LevelFilter {
    match byte {
        0 => LevelFilter::OFF,
        1 => LevelFilter::ERROR,
        2 => LevelFilter::WARN,
        3 => LevelFilter::INFO,
        4 => LevelFilter::DEBUG,
        _ => LevelFilter::TRACE,
    }
}

impl RtObs {
    pub fn new(id: u64) -> Arc<Self> {
        let filter = parse_filter(DEFAULT_FILTER).expect("the default parses");
        let max = max_level_of(&filter);
        Arc::new(RtObs {
            id,
            mode: AtomicU8::new(Mode::Off as u8),
            filter: FilterCell::new(FastFilter::from_targets(&filter)),
            max_level: AtomicU8::new(level_byte(max)),
            kept: Mutex::new(Vec::new()),
            registered: ArcSwap::from_pointee(None),
            in_callback: AtomicUsize::new(0),
            trace: TraceState::new(),
        })
    }

    pub fn mode(&self) -> Mode {
        match self.mode.load(Ordering::Acquire) {
            0 => Mode::Capture,
            1 => Mode::Off,
            2 => Mode::Drain,
            _ => Mode::Live,
        }
    }

    fn set_mode(&self, mode: Mode) {
        self.mode.store(mode as u8, Ordering::Release);
        // Interest is cached per callsite for every dispatcher at once: a mode change is a filter
        // change, so it is announced.
        tracing_core::callsite::rebuild_interest_cache();
    }

    fn max_level(&self) -> LevelFilter {
        match self.mode() {
            Mode::Capture => LevelFilter::DEBUG,
            Mode::Off => LevelFilter::OFF,
            _ => level_from_byte(self.max_level.load(Ordering::Relaxed)),
        }
    }

    /// Whether this runtime wants an event with this metadata.
    #[inline]
    pub fn wants(&self, meta: &Metadata<'_>) -> bool {
        if meta.is_span() {
            return self.trace.wants(meta);
        }
        match self.mode() {
            Mode::Capture => meta.is_event() && *meta.level() <= Level::DEBUG,
            Mode::Off => false,
            Mode::Drain | Mode::Live => self.filter.get().would_enable(meta.target(), meta.level()),
        }
    }

    pub fn begin_load(&self) {
        self.set_mode(Mode::Capture);
    }

    pub fn end_load(&self) {
        self.set_mode(Mode::Off);
    }

    /// Replaces the filter, keeping the one in force when the directives do not parse.
    pub fn set_filter(&self, directives: &str) -> Result<(), Refusal> {
        let parsed = parse_filter(directives).map_err(|_| Refusal::InvalidFilter)?;
        self.install(&parsed);
        // The swap is complete before the rebuild asks the subscriber again: a rebuild that read
        // the old filter would cache it.
        tracing_core::callsite::rebuild_interest_cache();
        Ok(())
    }

    /// What `set_filter` would do without telling `tracing`: for showing what the cache does.
    pub fn set_filter_without_rebuild(&self, directives: &str) -> Result<(), Refusal> {
        let parsed = parse_filter(directives).map_err(|_| Refusal::InvalidFilter)?;
        self.install(&parsed);
        Ok(())
    }

    fn install(&self, parsed: &Targets) {
        self.max_level
            .store(level_byte(max_level_of(parsed)), Ordering::Relaxed);
        self.filter.set(FastFilter::from_targets(parsed));
    }

    /// Registers the callback; `filter` replaces the loaded one when it is not empty. The events
    /// kept since `begin_load` are delivered here, on the calling thread, selected by the filter.
    pub fn set_log_callback(
        &self,
        callback: LogCallback,
        ctx: *mut c_void,
        filter: &str,
    ) -> Result<(), Refusal> {
        if !filter.is_empty() {
            let parsed = parse_filter(filter).map_err(|_| Refusal::InvalidFilter)?;
            self.install(&parsed);
        }
        self.registered.store(Arc::new(Some(Registered {
            callback,
            ctx: ctx as usize,
        })));
        self.set_mode(Mode::Drain);

        // Events that arrive while this runs queue behind the kept ones; the mode becomes Live
        // under the queue's lock, once the queue is empty.
        loop {
            let batch = {
                let mut kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
                if kept.is_empty() {
                    self.mode.store(Mode::Live as u8, Ordering::Release);
                    break;
                }
                std::mem::take(&mut *kept)
            };
            let filter = self.filter.get();
            for record in batch {
                if filter.would_enable(&record.target, &level_of(record.level)) {
                    self.deliver_owned(&record);
                }
            }
        }
        Ok(())
    }

    /// Removes the callback, waiting for the invocations under way; a host may then free its
    /// context. Refused from inside the callback, which would wait for itself.
    pub fn clear_log_callback(&self) -> Result<(), Refusal> {
        if IN_LOG_CALLBACK.with(|flag| flag.get()) {
            return Err(Refusal::FromInsideCallback);
        }
        self.set_mode(Mode::Off);
        self.registered.store(Arc::new(None));
        while self.in_callback.load(Ordering::Acquire) != 0 {
            std::thread::yield_now();
        }
        Ok(())
    }

    fn deliver_owned(&self, record: &OwnedRecord) {
        record.lend(|lent| self.call(lent));
    }

    #[inline]
    fn call(&self, record: &crate::record::LogRecord) {
        self.in_callback.fetch_add(1, Ordering::AcqRel);
        if let Some(registered) = &**self.registered.load() {
            IN_LOG_CALLBACK.with(|flag| flag.set(true));
            unsafe { (registered.callback)(registered.ctx as *mut c_void, record) };
            IN_LOG_CALLBACK.with(|flag| flag.set(false));
        }
        self.in_callback.fetch_sub(1, Ordering::AcqRel);
    }

    pub fn on_event(&self, event: &Event<'_>) {
        // An event logged from inside the callback, by an ak_* call it made, is dropped: delivering
        // it would re-enter the callback on its own stack, without bound.
        if IN_LOG_CALLBACK.with(|flag| flag.get()) {
            return;
        }
        match self.mode() {
            Mode::Off => {}
            Mode::Live => {
                // Re-checked: an event cached as wanted before a filter change may still arrive.
                if self.wants(event.metadata()) {
                    with_record(event, |record| self.call(record));
                }
            }
            Mode::Capture | Mode::Drain => {
                if !self.wants(event.metadata()) {
                    return;
                }
                let owned = with_record(event, |record| unsafe { OwnedRecord::copy_of(record) });
                let mut kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
                if self.mode() == Mode::Live {
                    drop(kept);
                    self.deliver_owned(&owned);
                } else {
                    kept.push(owned);
                }
            }
        }
    }

    pub fn kept_len(&self) -> usize {
        self.kept.lock().unwrap_or_else(PoisonError::into_inner).len()
    }

    pub fn interest(&self, meta: &Metadata<'_>) -> Interest {
        if self.wants(meta) {
            Interest::always()
        } else {
            Interest::never()
        }
    }
}

/// A bare subscriber: events only, spans refused.
pub struct RtSub(pub Arc<RtObs>);

impl Subscriber for RtSub {
    fn register_callsite(&self, meta: &'static Metadata<'static>) -> Interest {
        if meta.is_event() {
            self.0.interest(meta)
        } else {
            Interest::never()
        }
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(self.0.max_level())
    }

    fn enabled(&self, meta: &Metadata<'_>) -> bool {
        meta.is_event() && self.0.wants(meta)
    }

    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn enter(&self, _: &Id) {}
    fn exit(&self, _: &Id) {}

    fn event(&self, event: &Event<'_>) {
        self.0.on_event(event);
    }
}

/// The same state as a `Layer`, over a `Registry` that numbers and stores spans.
pub struct RtLayer(pub Arc<RtObs>);

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for RtLayer {
    fn register_callsite(&self, meta: &'static Metadata<'static>) -> Interest {
        self.0.interest(meta)
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        // The larger of what logs and spans ask for; spans are info or finer and rare.
        let spans = if self.0.trace.is_on() {
            LevelFilter::DEBUG
        } else {
            LevelFilter::OFF
        };
        Some(self.0.max_level().max(spans))
    }

    fn enabled(&self, meta: &Metadata<'_>, _: Context<'_, S>) -> bool {
        self.0.wants(meta)
    }

    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        self.0.on_event(event);
    }

    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        self.0.trace.on_new_span(attrs, id, ctx);
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        self.0.trace.on_close(id, ctx);
    }
}

pub fn bare_dispatch(obs: &Arc<RtObs>) -> Dispatch {
    Dispatch::new(RtSub(obs.clone()))
}

pub fn layered_dispatch(obs: &Arc<RtObs>) -> Dispatch {
    Dispatch::new(Registry::default().with(RtLayer(obs.clone())))
}
