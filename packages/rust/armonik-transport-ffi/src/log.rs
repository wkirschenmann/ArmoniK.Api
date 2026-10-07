//! The engine's logs, as a host receives them.
//!
//! The engine emits `tracing` events. A host that is not Rust has no subscriber for them, so this
//! library installs its own as the process's default dispatcher the first time a runtime is
//! created, and it routes each event to the log callback of the one runtime there is, or drops it
//! when there is none. Nothing is scoped to a thread: there is one runtime at a time, so one
//! callback, one filter, and no question of which runtime a thread belongs to.
//!
//! The filter is set when a runtime is created, from its `Logging.Filter` option, and never after.
//! `tracing` caches, per callsite, whether a subscriber wants it, so every change of the filter
//! rebuilds that cache; an event the filter does not select then costs a comparison of two
//! integers.
//!
//! The option is read from the same sources as the rest of the configuration, whose load logs: it
//! cannot be known while it runs. The load's events are therefore kept, on the thread that loads,
//! and delivered on it once the filter is known, before the creation returns.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::fmt::{self, Write as _};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Once, PoisonError, RwLock};

use tracing::field::{Field, Visit};
use tracing::level_filters::LevelFilter;
use tracing::span;
use tracing::subscriber::Interest;
use tracing::{Dispatch, Event, Level, Metadata, Subscriber};

use crate::abi::{
    ak_bytes_in, ak_log_callback, ak_log_field, ak_log_record, AK_LOG_DEBUG, AK_LOG_ERROR,
    AK_LOG_INFO, AK_LOG_TRACE, AK_LOG_WARN,
};

/// What `Logging.Filter` is layered over: warnings from every target, and the engine's own - the
/// targets that start with `armonik_transport` - at info.
pub(crate) const DEFAULT_FILTER: &str = "*=warn,armonik_transport*=info";

/// One directive of a filter: the events it covers, and the level it lets through.
#[derive(Clone, Debug, PartialEq)]
struct Directive {
    target: String,
    /// The target ended in `*`: the directive covers every target that starts with the text.
    /// Without it, it covers the target and the modules below it, by whole path segments.
    prefix: bool,
    level: LevelFilter,
}

impl Directive {
    fn covers(&self, target: &str) -> bool {
        match target.strip_prefix(self.target.as_str()) {
            None => false,
            Some(rest) => self.prefix || rest.is_empty() || rest.starts_with("::"),
        }
    }
}

/// Which events a runtime logs: a level for each directive's target, the most specific directive
/// deciding, and a level for what none covers.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Filter {
    /// Most specific first: the longest target, a whole-segment one before a `*` one of the same
    /// length.
    directives: Vec<Directive>,
    default: LevelFilter,
}

impl Filter {
    const fn off() -> Self {
        Self {
            directives: Vec::new(),
            default: LevelFilter::OFF,
        }
    }

    /// What the load of a configuration is held to: everything it could log, since the filter it
    /// will be selected by is among what it loads.
    const fn everything() -> Self {
        Self {
            directives: Vec::new(),
            default: LevelFilter::TRACE,
        }
    }

    /// The filter `text` states, over the default's: directive by directive, so that one stated
    /// for the same target - the same text, with or without its star - replaces the default's, and
    /// the rest of the default stands.
    ///
    /// The directives are `tracing`'s, as `EnvFilter` reads them but for what an event has no use
    /// for and for how a target matches: a level (`info`), a target and its level (`h2=debug`), or
    /// a target alone, which is every level of it. A target covers itself and the modules below it
    /// (`h2` covers `h2::proto`, not `h2x`), and one ending in `*` covers every target that
    /// starts with the text (`hyper*` covers `hyper` and `hyper_util`, and `*` alone every target,
    /// as a level alone does). Of the directives that cover an event the most specific decides. A
    /// directive whose level is not one, or that names a span or a field, is ignored and returned
    /// with the filter.
    pub(crate) fn parse(text: &str) -> (Self, Vec<String>) {
        let (default, mut directives, _) = Self::read(DEFAULT_FILTER);
        let (stated_default, stated, ignored) = Self::read(text);
        for directive in stated {
            directives
                .retain(|held| held.target != directive.target || held.prefix != directive.prefix);
            directives.push(directive);
        }
        directives.sort_by(|a, b| {
            b.target
                .len()
                .cmp(&a.target.len())
                .then(a.prefix.cmp(&b.prefix))
        });
        let filter = Self {
            directives,
            default: stated_default.or(default).unwrap_or(LevelFilter::OFF),
        };
        (filter, ignored)
    }

    /// The directives of `text`: the level for every target, the others, and what is ignored.
    fn read(text: &str) -> (Option<LevelFilter>, Vec<Directive>, Vec<String>) {
        let mut ignored = Vec::new();
        let mut default = None;
        let mut directives: Vec<Directive> = Vec::new();
        for directive in text.split(',').map(str::trim).filter(|d| !d.is_empty()) {
            if directive.contains(['[', '{']) {
                ignored.push(directive.to_owned());
                continue;
            }
            let (target, level) = match directive.split_once('=') {
                Some((target, level)) => (target.trim(), level_of_text(level.trim())),
                None => match level_of_text(directive) {
                    Some(level) => ("", Some(level)),
                    None => (directive, Some(LevelFilter::TRACE)),
                },
            };
            let Some(level) = level else {
                ignored.push(directive.to_owned());
                continue;
            };
            let (target, prefix) = match target.strip_suffix('*') {
                Some(text) => (text, true),
                None => (target, false),
            };
            if target.is_empty() {
                default = Some(level);
            } else {
                directives.retain(|stated| stated.target != target || stated.prefix != prefix);
                directives.push(Directive {
                    target: target.to_owned(),
                    prefix,
                    level,
                });
            }
        }
        (default, directives, ignored)
    }

    fn allows(&self, target: &str, level: Level) -> bool {
        let allowed = self
            .directives
            .iter()
            .find(|directive| directive.covers(target))
            .map_or(self.default, |directive| directive.level);
        level <= allowed
    }

    fn max_level(&self) -> LevelFilter {
        self.directives
            .iter()
            .map(|directive| directive.level)
            .chain([self.default])
            .max()
            .unwrap_or(LevelFilter::OFF)
    }
}

/// A level as a directive spells it. `LevelFilter` reads the empty text as a level, and a
/// directive with no level is not one.
fn level_of_text(text: &str) -> Option<LevelFilter> {
    if text.is_empty() {
        return None;
    }
    text.parse().ok()
}

type Callback = unsafe extern "C" fn(log_ctx: *mut c_void, record: *const ak_log_record);

/// Where the records go: the host's callback and the context it asked to be handed.
#[derive(Clone, Copy)]
pub(crate) struct Sink {
    callback: Callback,
    ctx: *mut c_void,
}

// SAFETY: the host promises the callback may run on any thread, and the context is only ever
// handed back to it.
unsafe impl Send for Sink {}
unsafe impl Sync for Sink {}

impl Sink {
    pub(crate) fn new(callback: ak_log_callback, ctx: *mut c_void) -> Option<Self> {
        callback.map(|callback| Self { callback, ctx })
    }
}

/// The runtime's sink, if it has one.
static SINK: RwLock<Option<Sink>> = RwLock::new(None);
/// How many threads have read `SINK` and not yet returned from the callback it named.
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static FILTER: RwLock<Filter> = RwLock::new(Filter::off());
static INSTALLED: Once = Once::new();

/// Taken by the unit tests that attach a sink or give a claim back, which detaches it.
#[cfg(test)]
pub(crate) static SINK_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

thread_local! {
    /// Set while this thread is inside the callback: an event it logs is dropped, since
    /// delivering it would enter the callback again on its own stack.
    static IN_CALLBACK: Cell<bool> = const { Cell::new(false) };
    /// The events a configuration's load has logged on this thread, while it loads.
    static LOADING: RefCell<Option<Vec<Kept>>> = const { RefCell::new(None) };
    static SCRATCH: RefCell<Scratch> = RefCell::new(Scratch::default());
}

fn filter() -> std::sync::RwLockReadGuard<'static, Filter> {
    FILTER.read().unwrap_or_else(PoisonError::into_inner)
}

/// Replaces the filter, and tells `tracing` when it changed: it caches each callsite's interest.
fn set_filter(replacement: Filter) {
    {
        let mut current = FILTER.write().unwrap_or_else(PoisonError::into_inner);
        if *current == replacement {
            return;
        }
        *current = replacement;
    }
    tracing_core::callsite::rebuild_interest_cache();
}

/// The process's default dispatcher, once. A process that already has another keeps it, and the
/// host receives nothing: whoever set it chose to read the engine's events itself.
fn install() {
    INSTALLED.call_once(|| {
        let _ = tracing::dispatcher::set_global_default(Dispatch::new(Router));
    });
}

struct Router;

impl Router {
    fn wants(meta: &Metadata<'_>) -> bool {
        meta.is_event() && filter().allows(meta.target(), *meta.level())
    }
}

impl Subscriber for Router {
    fn register_callsite(&self, meta: &'static Metadata<'static>) -> Interest {
        if Self::wants(meta) {
            Interest::always()
        } else {
            Interest::never()
        }
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(filter().max_level())
    }

    fn enabled(&self, meta: &Metadata<'_>) -> bool {
        Self::wants(meta)
    }

    // Spans are refused at their callsite, so none is ever built.
    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }

    fn record(&self, _: &span::Id, _: &span::Record<'_>) {}

    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}

    fn enter(&self, _: &span::Id) {}

    fn exit(&self, _: &span::Id) {}

    fn event(&self, event: &Event<'_>) {
        if IN_CALLBACK.with(Cell::get) {
            return;
        }
        // `try_with` throughout: an event logged while the thread's locals are being destroyed finds
        // them gone, and is delivered, or kept, as far as that allows.
        if LOADING
            .try_with(|loading| loading.borrow().is_some())
            .unwrap_or(false)
        {
            // Rendered with the borrow released: a value that logs while it renders comes back here.
            let kept = with_scratch(event, Kept::of);
            let _ = LOADING.try_with(|loading| {
                if let Some(events) = loading.borrow_mut().as_mut() {
                    events.push(kept);
                }
            });
        } else {
            with_scratch(event, |record, _| call(record));
        }
    }
}

/// Calls the host's callback, if there is one.
fn call(record: &ak_log_record) {
    let sink = {
        let attached = SINK.read().unwrap_or_else(PoisonError::into_inner);
        let Some(sink) = *attached else { return };
        // Counted before the read lock is given back, so a detach that takes the write lock next
        // finds this call to wait for.
        IN_FLIGHT.fetch_add(1, Ordering::AcqRel);
        sink
    };
    struct Leaving;
    impl Drop for Leaving {
        fn drop(&mut self) {
            IN_CALLBACK.with(|inside| inside.set(false));
            IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
        }
    }
    let _leaving = Leaving;
    IN_CALLBACK.with(|inside| inside.set(true));
    // SAFETY: the host promised the callback stays callable with its context until the runtime is
    // destroyed, and `detach` waits for the calls that are in flight.
    unsafe { (sink.callback)(sink.ctx, record) };
}

/// Stops delivering and waits for the deliveries under way, so that the host may free what its
/// context names. Called as the runtime's claim is given back.
///
/// Inside the callback it does not wait, since that would be for itself. The filter ends up
/// selecting nothing, so that an event with no runtime to receive it is not rendered.
pub(crate) fn detach() {
    *SINK.write().unwrap_or_else(PoisonError::into_inner) = None;
    if !IN_CALLBACK.with(Cell::get) {
        while IN_FLIGHT.load(Ordering::Acquire) != 0 {
            std::thread::yield_now();
        }
    }
    set_filter(Filter::off());
}

/// An event rendered where it can be kept: what a load logs before its filter is known.
struct Kept {
    level: u32,
    target: String,
    message: String,
    fields: Vec<(&'static str, String)>,
}

impl Kept {
    fn of(record: &ak_log_record, scratch: &Scratch) -> Self {
        // SAFETY: the record was built from `scratch`, which holds what it points at.
        let text = |view: ak_bytes_in| unsafe { as_str(view) }.to_owned();
        Self {
            level: record.level,
            target: text(record.target),
            message: text(record.message),
            fields: scratch
                .fields
                .iter()
                .zip(&scratch.keys)
                .map(|(field, key)| (*key, text(field.value)))
                .collect(),
        }
    }

    fn deliver(&self) {
        let fields: Vec<ak_log_field> = self
            .fields
            .iter()
            .map(|(key, value)| ak_log_field {
                key: view(key),
                value: view(value),
            })
            .collect();
        call(&build_record(
            self.level,
            view(&self.target),
            view(&self.message),
            &fields,
        ));
    }
}

/// The text a view holds.
///
/// # Safety
///
/// The view must be of valid UTF-8 that is still alive.
unsafe fn as_str<'a>(view: ak_bytes_in) -> &'a str {
    if view.len == 0 {
        return "";
    }
    // SAFETY: the caller's promise.
    unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(view.ptr, view.len)) }
}

fn view(text: &str) -> ak_bytes_in {
    ak_bytes_in {
        ptr: text.as_ptr(),
        len: text.len(),
    }
}

fn build_record(
    level: u32,
    target: ak_bytes_in,
    message: ak_bytes_in,
    fields: &[ak_log_field],
) -> ak_log_record {
    ak_log_record {
        struct_size: std::mem::size_of::<ak_log_record>() as u32,
        level,
        target,
        message,
        field_count: fields.len(),
        fields: if fields.is_empty() {
            std::ptr::null()
        } else {
            fields.as_ptr()
        },
    }
}

fn level_code(level: &Level) -> u32 {
    match *level {
        Level::ERROR => AK_LOG_ERROR,
        Level::WARN => AK_LOG_WARN,
        Level::INFO => AK_LOG_INFO,
        Level::DEBUG => AK_LOG_DEBUG,
        Level::TRACE => AK_LOG_TRACE,
    }
}

fn level_of(code: u32) -> Level {
    match code {
        AK_LOG_ERROR => Level::ERROR,
        AK_LOG_WARN => Level::WARN,
        AK_LOG_INFO => Level::INFO,
        AK_LOG_DEBUG => Level::DEBUG,
        _ => Level::TRACE,
    }
}

/// The buffers an event is rendered into, reused from one event to the next on a thread: a value
/// is rendered once into `text`, and the fields are views of it.
#[derive(Default)]
struct Scratch {
    text: String,
    /// Where each field's value is in `text`, in the order the event names them.
    parts: Vec<(u32, u32)>,
    keys: Vec<&'static str>,
    message: (u32, u32),
    fields: Vec<ak_log_field>,
}

impl Scratch {
    fn clear(&mut self) {
        self.text.clear();
        self.parts.clear();
        self.keys.clear();
        self.fields.clear();
        self.message = (0, 0);
    }
}

struct Render<'a>(&'a mut Scratch);

impl Render<'_> {
    fn put(&mut self, field: &Field, write: impl FnOnce(&mut String) -> fmt::Result) {
        let start = self.0.text.len();
        // A value whose rendering fails is kept as far as it got.
        let _ = write(&mut self.0.text);
        let part = (start as u32, self.0.text.len() as u32);
        if field.name() == "message" {
            self.0.message = part;
        } else {
            self.0.keys.push(field.name());
            self.0.parts.push(part);
        }
    }
}

impl Visit for Render<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.put(field, |text| write!(text, "{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.put(field, |text| {
            text.push_str(value);
            Ok(())
        });
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.put(field, |text| write!(text, "{value}"));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.put(field, |text| write!(text, "{value}"));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.put(field, |text| write!(text, "{value}"));
    }
}

/// Renders `event` and hands the record to `then`, with the buffers it points into.
///
/// An event logged while another is being rendered on the thread, by a value's own rendering, or
/// while the thread's locals are being destroyed, finds the buffers taken or gone and renders into
/// ones of its own.
fn with_scratch<R>(event: &Event<'_>, then: impl FnOnce(&ak_log_record, &Scratch) -> R) -> R {
    let mut then = Some(then);
    let shared = SCRATCH.try_with(|cell| {
        let mut scratch = cell.try_borrow_mut().ok()?;
        Some(rendered(&mut scratch, event, then.take()?))
    });
    match shared {
        Ok(Some(done)) => done,
        _ => rendered(
            &mut Scratch::default(),
            event,
            then.expect("a render that did not run leaves its continuation"),
        ),
    }
}

fn rendered<R>(
    scratch: &mut Scratch,
    event: &Event<'_>,
    then: impl FnOnce(&ak_log_record, &Scratch) -> R,
) -> R {
    scratch.clear();
    event.record(&mut Render(scratch));

    // The text has stopped growing, so its views stay where they are.
    let text = scratch.text.as_str();
    let slice = |(start, end): (u32, u32)| view(&text[start as usize..end as usize]);
    let message = slice(scratch.message);
    scratch
        .fields
        .extend(
            scratch
                .keys
                .iter()
                .zip(&scratch.parts)
                .map(|(key, part)| ak_log_field {
                    key: view(key),
                    value: slice(*part),
                }),
        );
    let meta = event.metadata();
    let record = build_record(
        level_code(meta.level()),
        view(meta.target()),
        message,
        &scratch.fields,
    );
    then(&record, scratch)
}

/// What a runtime's creation holds while its configuration loads, and settles once it has.
pub(crate) struct Loading {
    capturing: bool,
}

/// Attaches the runtime's sink, and starts keeping what the load logs on this thread.
///
/// Without a sink nothing is logged, nothing is kept, and the filter selects nothing, so that a
/// host that wants no logs pays no more for them than an integer comparison per event.
///
/// Until `settle`, the filter selects everything: the engine has no other thread then, so the
/// loading thread's events are the only ones, and they are kept rather than delivered.
pub(crate) fn begin(sink: Option<Sink>) -> Loading {
    install();
    let capturing = sink.is_some();
    *SINK.write().unwrap_or_else(PoisonError::into_inner) = sink;
    set_filter(if capturing {
        Filter::everything()
    } else {
        Filter::off()
    });
    if capturing {
        LOADING.with(|loading| *loading.borrow_mut() = Some(Vec::new()));
    }
    Loading { capturing }
}

impl Loading {
    /// Sets the filter the load found, or the default, and delivers what the load logged and the
    /// filter selects, on this thread.
    pub(crate) fn settle(self, filter: Option<&str>) {
        if !self.capturing {
            return;
        }
        let kept = LOADING
            .with(|loading| loading.borrow_mut().take())
            .unwrap_or_default();
        let (filter, ignored) = Filter::parse(filter.unwrap_or_default());
        set_filter(filter.clone());
        for event in kept
            .iter()
            .filter(|event| filter.allows(&event.target, level_of(event.level)))
        {
            event.deliver();
        }
        // Whatever the filter says: it is the filter the warning is about.
        for directive in ignored {
            Kept {
                level: AK_LOG_WARN,
                target: "armonik_transport_ffi::log".to_owned(),
                message:
                    "the log filter holds a directive that is not understood, which is ignored"
                        .to_owned(),
                fields: vec![("directive", directive)],
            }
            .deliver();
        }
    }
}

impl Drop for Loading {
    fn drop(&mut self) {
        if self.capturing {
            let _ = LOADING.try_with(|loading| loading.borrow_mut().take());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allows(filter: &Filter, target: &str, level: Level) -> bool {
        filter.allows(target, level)
    }

    /// What lets a host free the context its callback was given: no call is under way once the
    /// detach has returned. It attaches the process's sink, so it takes `SINK_TESTS`.
    #[test]
    fn a_detach_waits_for_the_delivery_under_way() {
        use std::sync::atomic::AtomicBool;

        static ENTERED: AtomicBool = AtomicBool::new(false);
        static RELEASED: AtomicBool = AtomicBool::new(false);
        static DETACHED: AtomicBool = AtomicBool::new(false);

        unsafe extern "C" fn blocking(_: *mut c_void, _: *const ak_log_record) {
            ENTERED.store(true, Ordering::SeqCst);
            while !RELEASED.load(Ordering::SeqCst) {
                std::thread::yield_now();
            }
        }

        let _alone = SINK_TESTS.lock().unwrap_or_else(PoisonError::into_inner);
        *SINK.write().unwrap_or_else(PoisonError::into_inner) =
            Sink::new(Some(blocking), std::ptr::null_mut());
        let delivering = std::thread::spawn(|| {
            call(&build_record(
                AK_LOG_INFO,
                view("target"),
                view("message"),
                &[],
            ))
        });
        while !ENTERED.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }

        let detaching = std::thread::spawn(|| {
            detach();
            DETACHED.store(true, Ordering::SeqCst);
        });
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(
            !DETACHED.load(Ordering::SeqCst),
            "the detach returned while the callback was still running"
        );

        RELEASED.store(true, Ordering::SeqCst);
        delivering.join().expect("the delivery ends");
        detaching.join().expect("the detach ends");
        assert!(DETACHED.load(Ordering::SeqCst));
        assert!(SINK
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .is_none());
    }

    #[test]
    fn the_default_is_warnings_everywhere_and_the_engines_own_events_at_info() {
        let (filter, ignored) = Filter::parse("");
        assert!(ignored.is_empty());
        assert_eq!(filter, Filter::parse(DEFAULT_FILTER).0);
        for engine in [
            "armonik_transport::grpc",
            "armonik_transport_ffi::config",
            "armonik_transport_ffi::log",
        ] {
            assert!(allows(&filter, engine, Level::INFO), "{engine}");
            assert!(!allows(&filter, engine, Level::DEBUG), "{engine}");
        }
        for library in [
            "h2::proto::connection",
            "hyper_util::client",
            "hyper_rustls",
            "tonic::transport",
            "tower::buffer",
            "something::else",
        ] {
            assert!(allows(&filter, library, Level::WARN), "{library}");
            assert!(!allows(&filter, library, Level::INFO), "{library}");
        }
        assert_eq!(filter.max_level(), LevelFilter::INFO);
    }

    #[test]
    fn a_directive_stated_for_a_target_is_layered_over_the_default_and_the_rest_stands() {
        let (filter, ignored) = Filter::parse("armonik_transport=debug");
        assert!(ignored.is_empty());
        assert!(allows(&filter, "armonik_transport::grpc", Level::DEBUG));
        assert!(!allows(&filter, "armonik_transport::grpc", Level::TRACE));
        // The default's star for the same text covers what the segment directive does not.
        assert!(allows(
            &filter,
            "armonik_transport_ffi::config",
            Level::INFO
        ));
        assert!(!allows(
            &filter,
            "armonik_transport_ffi::config",
            Level::DEBUG
        ));
        assert!(allows(&filter, "h2::proto", Level::WARN));
        assert!(!allows(&filter, "h2::proto", Level::INFO));

        // The same target, star included, replaces the default's directive.
        let (filter, _) = Filter::parse("armonik_transport*=debug");
        assert!(allows(
            &filter,
            "armonik_transport_ffi::config",
            Level::DEBUG
        ));
        assert!(allows(&filter, "armonik_transport::grpc", Level::DEBUG));
        assert!(!allows(&filter, "h2::proto", Level::INFO));
    }

    #[test]
    fn a_user_turns_the_default_down_or_off_by_stating_its_directives() {
        // The star's level and the engine's are two directives of the default, and each is
        // replaced by stating its own.
        let (filter, _) = Filter::parse("*=error,armonik_transport*=error");
        assert!(allows(&filter, "armonik_transport::grpc", Level::ERROR));
        assert!(!allows(&filter, "armonik_transport::grpc", Level::WARN));
        assert!(!allows(&filter, "h2::proto", Level::WARN));

        let (off, _) = Filter::parse("*=off,armonik_transport*=off");
        assert!(!allows(&off, "armonik_transport::grpc", Level::ERROR));
        assert!(!allows(&off, "h2::proto", Level::ERROR));
        assert_eq!(off.max_level(), LevelFilter::OFF);

        // Stating the star alone leaves the engine's directive standing.
        let (star_only, _) = Filter::parse("*=off");
        assert!(allows(&star_only, "armonik_transport::grpc", Level::INFO));
        assert!(!allows(&star_only, "h2::proto", Level::ERROR));
    }

    #[test]
    fn a_target_covers_its_own_path_segments_and_not_a_longer_name() {
        let (filter, _) = Filter::parse("h2=debug");
        assert!(allows(&filter, "h2", Level::DEBUG));
        assert!(allows(&filter, "h2::proto::connection", Level::DEBUG));
        assert!(!allows(&filter, "h2x", Level::DEBUG));
        assert!(!allows(&filter, "h2x::proto", Level::DEBUG));
        assert!(allows(&filter, "h2x", Level::WARN));
    }

    #[test]
    fn a_target_ending_in_a_star_covers_every_target_that_starts_with_it() {
        let (filter, _) = Filter::parse("hyper*=debug");
        assert!(allows(&filter, "hyper", Level::DEBUG));
        assert!(allows(&filter, "hyper_util::client::legacy", Level::DEBUG));
        assert!(!allows(&filter, "h2", Level::DEBUG));

        let (filter, _) = Filter::parse("info,hyper*=error");
        assert!(!allows(&filter, "hyper", Level::WARN));
        assert!(!allows(&filter, "hyper_util::client::legacy", Level::WARN));
        assert!(allows(&filter, "hyper_util::client::legacy", Level::ERROR));
        assert!(allows(&filter, "something::else", Level::INFO));
    }

    #[test]
    fn a_star_alone_covers_every_target_and_is_the_least_specific() {
        let (filter, ignored) = Filter::parse("*=debug,h2=error");
        assert!(ignored.is_empty());
        assert!(allows(&filter, "anything::at_all", Level::DEBUG));
        assert!(!allows(&filter, "anything::at_all", Level::TRACE));
        assert!(allows(&filter, "h2::proto", Level::ERROR));
        assert!(!allows(&filter, "h2::proto", Level::WARN));
        // The default's directive for the engine is more specific, and stands.
        assert!(!allows(&filter, "armonik_transport::grpc", Level::DEBUG));

        let (everything, _) = Filter::parse("*");
        assert!(allows(&everything, "any", Level::TRACE));
        assert_eq!(Filter::parse("*=info").0, Filter::parse("info").0);
    }

    #[test]
    fn the_most_specific_directive_decides_and_a_segment_beats_a_star_of_the_same_length() {
        let (filter, _) =
            Filter::parse("error,armonik*=info,armonik_transport::grpc=trace,h2=debug,h2*=warn");
        // The longer target.
        assert!(allows(
            &filter,
            "armonik_transport::grpc::channel",
            Level::TRACE
        ));
        assert!(!allows(&filter, "armonik_transport::http2", Level::DEBUG));
        assert!(allows(&filter, "armonik_transport::http2", Level::INFO));
        // The same length: the segment one decides where both cover.
        assert!(allows(&filter, "h2", Level::DEBUG));
        assert!(allows(&filter, "h2::proto", Level::DEBUG));
        // Only the star covers a longer name.
        assert!(!allows(&filter, "h2x", Level::INFO));
        assert!(allows(&filter, "h2x", Level::WARN));
        // The order the directives are written in decides nothing.
        let (reversed, _) = Filter::parse("h2*=warn,h2=debug");
        assert_eq!(
            reversed.allows("h2", Level::DEBUG),
            filter.allows("h2", Level::DEBUG)
        );
    }

    #[test]
    fn the_longest_target_decides() {
        let (filter, _) =
            Filter::parse("warn,armonik_transport=info,armonik_transport::grpc=trace");
        assert!(allows(
            &filter,
            "armonik_transport::grpc::channel",
            Level::TRACE
        ));
        assert!(allows(&filter, "armonik_transport::http2", Level::INFO));
        assert!(!allows(&filter, "armonik_transport::http2", Level::DEBUG));
        assert!(!allows(&filter, "other", Level::INFO));
        assert_eq!(filter.max_level(), LevelFilter::TRACE);
    }

    #[test]
    fn a_target_alone_is_all_its_levels_and_the_default_stands_for_the_rest() {
        let (filter, _) = Filter::parse("h2");
        assert!(allows(&filter, "h2", Level::TRACE));
        assert!(allows(&filter, "armonik_transport", Level::INFO));
        assert!(!allows(&filter, "armonik_transport", Level::DEBUG));
    }

    #[test]
    fn a_stray_word_switches_nothing_off() {
        // A word that is not a level is a target: one nothing emits, which selects nothing.
        for word in ["Information", "Warning", "inf", "Information,Warning"] {
            let (filter, ignored) = Filter::parse(word);
            assert!(ignored.is_empty(), "{word}");
            assert!(
                allows(&filter, "armonik_transport::grpc", Level::INFO),
                "{word}"
            );
            assert!(allows(&filter, "h2::proto", Level::WARN), "{word}");
        }
    }

    #[test]
    fn a_directive_that_names_nothing_the_engine_emits_is_no_error() {
        let (filter, ignored) = Filter::parse("info,no_such_crate=trace");
        assert!(ignored.is_empty());
        assert!(allows(&filter, "armonik_transport", Level::INFO));
        assert!(allows(&filter, "no_such_crate", Level::TRACE));
    }

    #[test]
    fn a_directive_that_is_not_understood_is_ignored_and_the_rest_kept() {
        let (filter, ignored) = Filter::parse("info, h2=loud, h2[conn]=debug, hyper{x=1}=trace");
        assert_eq!(ignored, ["h2=loud", "h2[conn]=debug", "hyper{x=1}=trace"]);
        assert!(allows(&filter, "something::else", Level::INFO));
        assert!(!allows(&filter, "h2", Level::DEBUG));
    }

    #[test]
    fn a_filter_none_of_whose_directives_holds_is_the_default() {
        let (filter, ignored) = Filter::parse("h2=loud");
        assert_eq!(ignored, ["h2=loud"]);
        assert_eq!(filter, Filter::parse(DEFAULT_FILTER).0);
        assert_eq!(Filter::parse("").0, Filter::parse(DEFAULT_FILTER).0);
    }

    #[test]
    fn a_target_stated_twice_takes_the_later_level() {
        let (filter, _) = Filter::parse("h2=debug,h2=error,warn");
        assert!(!allows(&filter, "h2", Level::WARN));
        assert!(allows(&filter, "h2", Level::ERROR));
    }

    #[test]
    fn levels_are_read_without_regard_to_case() {
        let (filter, ignored) = Filter::parse("INFO,H2=Debug");
        assert!(ignored.is_empty());
        assert!(allows(&filter, "something::else", Level::INFO));
        assert!(
            !allows(&filter, "h2", Level::DEBUG),
            "targets keep their case"
        );
    }
}
