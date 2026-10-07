//! Point 4: the engine's spans, built only while a trace callback is registered and only those
//! configured, and delivered as one record when each ends.
//!
//! The runtime's dispatcher is a `Registry` with `RtLayer` on it: the registry numbers and stores
//! the spans, the layer reads a span's identifiers at its start and delivers its record at its
//! close. What decides whether a span exists is `TraceState::wants`, asked of the dispatcher
//! before the span is built.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use arc_swap::ArcSwap;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id};
use tracing::{Metadata, Subscriber};
use tracing_subscriber::filter::Targets;

use crate::fast_filter::{FastFilter, FilterCell};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

use crate::record::{BytesIn, LogField};

#[repr(C)]
pub struct TraceRecord {
    pub struct_size: u32,
    pub status: u32,
    pub trace_id: [u8; 16],
    pub span_id: [u8; 8],
    pub parent_span_id: [u8; 8],
    pub name: BytesIn,
    pub start_unix_ns: u64,
    pub end_unix_ns: u64,
    pub attr_count: u32,
    pub reserved: u32,
    pub attrs: *const LogField,
}

pub type TraceCallback = unsafe extern "C" fn(ctx: *mut c_void, record: *const TraceRecord);

/// What a runtime has been told about its traces.
pub struct TraceState {
    on: AtomicBool,
    /// Which spans are built: the same `target=level` syntax as the log filter, over the spans'
    /// targets. Empty means none.
    spans: FilterCell,
    callback: ArcSwap<Option<(TraceCallback, usize)>>,
}

impl TraceState {
    pub fn new() -> Self {
        TraceState {
            on: AtomicBool::new(false),
            spans: FilterCell::new(FastFilter::off()),
            callback: ArcSwap::from_pointee(None),
        }
    }

    pub fn register(&self, callback: TraceCallback, ctx: *mut c_void, spans: &str) -> Result<(), ()> {
        let parsed = spans.parse::<Targets>().map_err(|_| ())?;
        self.spans.set(FastFilter::from_targets(&parsed));
        self.callback.store(Arc::new(Some((callback, ctx as usize))));
        self.on.store(true, Ordering::Release);
        tracing_core::callsite::rebuild_interest_cache();
        Ok(())
    }

    pub fn unregister(&self) {
        self.on.store(false, Ordering::Release);
        self.callback.store(Arc::new(None));
        tracing_core::callsite::rebuild_interest_cache();
    }

    pub fn is_on(&self) -> bool {
        self.on.load(Ordering::Acquire)
    }

    #[inline]
    pub fn wants(&self, meta: &Metadata<'_>) -> bool {
        self.on.load(Ordering::Acquire)
            && meta.is_span()
            && self.spans.get().would_enable(meta.target(), meta.level())
    }
}

impl Default for TraceState {
    fn default() -> Self {
        Self::new()
    }
}

/// What a span keeps from its start to its close.
struct SpanData {
    trace_id: [u8; 16],
    span_id: [u8; 8],
    parent_span_id: [u8; 8],
    start: Instant,
    start_unix_ns: u64,
    text: String,
    attrs: Vec<(&'static str, u32, u32)>,
}

/// Reads the fields a span starts with: the trace identifiers the call's context gave it, and the
/// attributes the record carries.
struct Start<'a> {
    data: &'a mut SpanData,
    has_trace: bool,
}

impl Visit for Start<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write;
        let start = self.data.text.len() as u32;
        let _ = write!(self.data.text, "{value:?}");
        self.data
            .attrs
            .push((field.name(), start, self.data.text.len() as u32));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        let start = self.data.text.len() as u32;
        self.data.text.push_str(value);
        self.data
            .attrs
            .push((field.name(), start, self.data.text.len() as u32));
    }

    fn record_u128(&mut self, field: &Field, value: u128) {
        if field.name() == "trace_id" {
            self.data.trace_id = value.to_be_bytes();
            self.has_trace = true;
        } else {
            self.record_debug(field, &value);
        }
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == "parent_span_id" {
            self.data.parent_span_id = value.to_be_bytes();
        } else {
            self.record_debug(field, &value);
        }
    }
}

fn unix_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64)
}

impl TraceState {
    pub fn on_new_span<S: Subscriber + for<'a> LookupSpan<'a>>(
        &self,
        attrs: &Attributes<'_>,
        id: &Id,
        ctx: Context<'_, S>,
    ) {
        let Some(span) = ctx.span(id) else { return };
        let mut data = SpanData {
            trace_id: [0; 16],
            span_id: fastrand::u64(..).to_be_bytes(),
            parent_span_id: [0; 8],
            start: Instant::now(),
            start_unix_ns: unix_ns(),
            text: String::new(),
            attrs: Vec::new(),
        };
        let mut start = Start {
            data: &mut data,
            has_trace: false,
        };
        attrs.record(&mut start);
        if !start.has_trace {
            // A child: the trace and the parent come from the enclosing span.
            if let Some(parent) = span.parent() {
                if let Some(parent_data) = parent.extensions().get::<SpanData>() {
                    data.trace_id = parent_data.trace_id;
                    data.parent_span_id = parent_data.span_id;
                }
            }
        }
        span.extensions_mut().insert(data);
    }

    pub fn on_close<S: Subscriber + for<'a> LookupSpan<'a>>(&self, id: Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(&id) else { return };
        let extensions = span.extensions();
        let Some(data) = extensions.get::<SpanData>() else {
            return;
        };
        let guard = self.callback.load();
        let Some((callback, ctx_ptr)) = &**guard else {
            return;
        };
        let end_unix_ns = data.start_unix_ns + data.start.elapsed().as_nanos() as u64;
        let attrs: Vec<LogField> = data
            .attrs
            .iter()
            .map(|(key, start, end)| LogField {
                key: BytesIn {
                    ptr: key.as_ptr(),
                    len: key.len(),
                },
                value: BytesIn {
                    ptr: data.text[*start as usize..*end as usize].as_ptr(),
                    len: (*end - *start) as usize,
                },
            })
            .collect();
        let name = span.name();
        let record = TraceRecord {
            struct_size: std::mem::size_of::<TraceRecord>() as u32,
            status: 0,
            trace_id: data.trace_id,
            span_id: data.span_id,
            parent_span_id: data.parent_span_id,
            name: BytesIn {
                ptr: name.as_ptr(),
                len: name.len(),
            },
            start_unix_ns: data.start_unix_ns,
            end_unix_ns,
            attr_count: attrs.len() as u32,
            reserved: 0,
            attrs: attrs.as_ptr(),
        };
        unsafe { callback(*ctx_ptr as *mut c_void, &record) };
    }
}
