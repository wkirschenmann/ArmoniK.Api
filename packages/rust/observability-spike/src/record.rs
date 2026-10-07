//! Point 3: an event's fields rendered into a C record valid for the callback alone.
//!
//! The record's shape mirrors observability.md's sketch. `key` points at the field's static name
//! and is never copied; every value is rendered once into a per-thread buffer that is reused from
//! one event to the next, so a steady state allocates nothing.

use std::cell::RefCell;
use std::ffi::c_void;
use std::fmt::{self, Write};

use tracing::field::{Field, Visit};
use tracing::{Event, Level};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct BytesIn {
    pub ptr: *const u8,
    pub len: usize,
}

impl BytesIn {
    pub const EMPTY: BytesIn = BytesIn {
        ptr: std::ptr::null(),
        len: 0,
    };

    fn of(text: &str) -> Self {
        BytesIn {
            ptr: text.as_ptr(),
            len: text.len(),
        }
    }

    /// # Safety
    /// The bytes must still be alive: inside the callback.
    pub unsafe fn as_str(&self) -> &str {
        if self.ptr.is_null() {
            ""
        } else {
            unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(self.ptr, self.len)) }
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LogField {
    pub key: BytesIn,
    pub value: BytesIn,
}

pub const AK_LOG_ERROR: u32 = 1;
pub const AK_LOG_WARN: u32 = 2;
pub const AK_LOG_INFO: u32 = 3;
pub const AK_LOG_DEBUG: u32 = 4;
pub const AK_LOG_TRACE: u32 = 5;

#[repr(C)]
pub struct LogRecord {
    /// Sizeof the structure the library built, so that a host reads only what exists.
    pub struct_size: u32,
    pub level: u32,
    pub field_count: u32,
    pub reserved: u32,
    pub target: BytesIn,
    pub message: BytesIn,
    pub fields: *const LogField,
}

pub type LogCallback = unsafe extern "C" fn(ctx: *mut c_void, record: *const LogRecord);

pub fn level_code(level: &Level) -> u32 {
    match *level {
        Level::ERROR => AK_LOG_ERROR,
        Level::WARN => AK_LOG_WARN,
        Level::INFO => AK_LOG_INFO,
        Level::DEBUG => AK_LOG_DEBUG,
        Level::TRACE => AK_LOG_TRACE,
    }
}

/// Offsets into `text`, resolved to pointers once rendering has stopped growing it.
struct Span {
    key: &'static str,
    start: u32,
    end: u32,
}

/// The buffers an event is rendered into.
#[derive(Default)]
pub struct Scratch {
    text: String,
    spans: Vec<Span>,
    message: Option<(u32, u32)>,
    fields: Vec<LogField>,
}

impl Scratch {
    fn clear(&mut self) {
        self.text.clear();
        self.spans.clear();
        self.fields.clear();
        self.message = None;
    }
}

struct Render<'a>(&'a mut Scratch);

impl Render<'_> {
    fn put(&mut self, field: &Field, write: impl FnOnce(&mut String) -> fmt::Result) {
        let start = self.0.text.len() as u32;
        // A value whose Display fails is kept as far as it got.
        let _ = write(&mut self.0.text);
        let end = self.0.text.len() as u32;
        if field.name() == "message" {
            self.0.message = Some((start, end));
        } else {
            self.0.spans.push(Span {
                key: field.name(),
                start,
                end,
            });
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

thread_local! {
    static SCRATCH: RefCell<Scratch> = RefCell::new(Scratch::default());
}

/// Renders `event` and hands the record to `deliver`, which must not keep it.
///
/// A callback that logs again on the same thread - it called an `ak_*` function - finds the
/// buffer taken and renders into one of its own: re-entry is rare and costs an allocation.
pub fn with_record<R>(event: &Event<'_>, deliver: impl FnOnce(&LogRecord) -> R) -> R {
    SCRATCH.with(|cell| match cell.try_borrow_mut() {
        Ok(mut scratch) => render(&mut scratch, event, deliver),
        Err(_) => render(&mut Scratch::default(), event, deliver),
    })
}

fn render<R>(scratch: &mut Scratch, event: &Event<'_>, deliver: impl FnOnce(&LogRecord) -> R) -> R {
    scratch.clear();
    event.record(&mut Render(scratch));

    let text = scratch.text.as_str();
    let message = match scratch.message {
        Some((start, end)) => BytesIn::of(&text[start as usize..end as usize]),
        None => BytesIn::EMPTY,
    };
    scratch.fields.extend(scratch.spans.iter().map(|span| LogField {
        key: BytesIn::of(span.key),
        value: BytesIn::of(&text[span.start as usize..span.end as usize]),
    }));
    let meta = event.metadata();
    let record = LogRecord {
        struct_size: std::mem::size_of::<LogRecord>() as u32,
        level: level_code(meta.level()),
        field_count: scratch.fields.len() as u32,
        reserved: 0,
        target: BytesIn::of(meta.target()),
        message,
        fields: scratch.fields.as_ptr(),
    };
    deliver(&record)
}

/// A record that outlives its event: what is kept for a callback not yet registered.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnedRecord {
    pub level: u32,
    pub target: String,
    pub message: String,
    pub fields: Vec<(String, String)>,
}

impl OwnedRecord {
    /// # Safety
    /// `record` must be valid: inside the callback.
    pub unsafe fn copy_of(record: &LogRecord) -> Self {
        let fields = unsafe { std::slice::from_raw_parts(record.fields, record.field_count as usize) };
        OwnedRecord {
            level: record.level,
            target: unsafe { record.target.as_str() }.to_owned(),
            message: unsafe { record.message.as_str() }.to_owned(),
            fields: fields
                .iter()
                .map(|field| unsafe { (field.key.as_str().to_owned(), field.value.as_str().to_owned()) })
                .collect(),
        }
    }

    /// Borrowed as a record again, for delivering what was kept.
    pub fn lend<R>(&self, deliver: impl FnOnce(&LogRecord) -> R) -> R {
        let fields: Vec<LogField> = self
            .fields
            .iter()
            .map(|(key, value)| LogField {
                key: BytesIn::of(key),
                value: BytesIn::of(value),
            })
            .collect();
        let record = LogRecord {
            struct_size: std::mem::size_of::<LogRecord>() as u32,
            level: self.level,
            field_count: fields.len() as u32,
            reserved: 0,
            target: BytesIn::of(&self.target),
            message: BytesIn::of(&self.message),
            fields: fields.as_ptr(),
        };
        deliver(&record)
    }
}

/// The level a kept record is selected by: the same ordering a `LevelFilter` has.
pub fn level_of(code: u32) -> Level {
    match code {
        AK_LOG_ERROR => Level::ERROR,
        AK_LOG_WARN => Level::WARN,
        AK_LOG_INFO => Level::INFO,
        AK_LOG_DEBUG => Level::DEBUG,
        _ => Level::TRACE,
    }
}
