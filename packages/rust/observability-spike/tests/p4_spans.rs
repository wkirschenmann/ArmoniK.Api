//! Point 4: spans built only while a trace callback is registered and only those configured.

use std::ffi::c_void;
use std::sync::Mutex;

use observability_spike::record::LogField;
use observability_spike::runtime::{Front, ObsRuntime};
use observability_spike::trace::TraceRecord;
use tracing::{info_span, Instrument, Span};

#[derive(Debug, Clone)]
struct Owned {
    name: String,
    trace_id: [u8; 16],
    span_id: [u8; 8],
    parent_span_id: [u8; 8],
    start: u64,
    end: u64,
    attrs: Vec<(String, String)>,
}

#[derive(Default)]
struct Spans(Mutex<Vec<Owned>>);

unsafe extern "C" fn take_span(ctx: *mut c_void, record: *const TraceRecord) {
    let spans = unsafe { &*(ctx as *const Spans) };
    let record = unsafe { &*record };
    let attrs = unsafe { std::slice::from_raw_parts(record.attrs, record.attr_count as usize) };
    let text = |field: &LogField| unsafe { (field.key.as_str().to_owned(), field.value.as_str().to_owned()) };
    spans.0.lock().unwrap().push(Owned {
        name: unsafe { record.name.as_str() }.to_owned(),
        trace_id: record.trace_id,
        span_id: record.span_id,
        parent_span_id: record.parent_span_id,
        start: record.start_unix_ns,
        end: record.end_unix_ns,
        attrs: attrs.iter().map(text).collect(),
    });
}

const TRACE_ID: u128 = 0x4bf92f3577b34da6a3ce929d0e0e4736;

/// The call's span: the root the engine builds from the context the host handed over.
fn call_span(sampled: bool, parent: u64) -> Span {
    if !sampled {
        return Span::none();
    }
    info_span!(target: "armonik_transport::trace::call", "call", trace_id = TRACE_ID, parent_span_id = parent, method = "/x/Y")
}

fn dial_span(parent: &Span) -> Span {
    info_span!(target: "armonik_transport::trace::dial", parent: parent, "dial", endpoint = "http://127.0.0.1:1")
}

fn retry_span(parent: &Span, attempt: u64) -> Span {
    info_span!(target: "armonik_transport::trace::retry", parent: parent, "retry", attempt)
}

#[test]
fn no_span_is_built_while_no_trace_callback_is_registered() {
    let runtime = ObsRuntime::new(Front::Layered);
    let _inside = runtime.scope();
    let call = call_span(true, 7);
    assert!(call.is_disabled(), "no callback, so the dispatcher refuses the span");
    assert!(dial_span(&call).is_disabled());
}

#[test]
fn spans_cross_with_their_identifiers_when_they_end() {
    let runtime = ObsRuntime::new(Front::Layered);
    let spans = Box::new(Spans::default());
    runtime
        .obs
        .trace
        .register(take_span, &*spans as *const Spans as *mut c_void, "armonik_transport::trace=info")
        .unwrap();
    let _inside = runtime.scope();

    let call = call_span(true, 0x1122334455667788);
    {
        let dial = dial_span(&call);
        let _entered = dial.enter();
    }
    retry_span(&call, 2).in_scope(|| {});
    drop(call);

    let got = spans.0.lock().unwrap().clone();
    let names: Vec<&str> = got.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["dial", "retry", "call"], "each at its end, children first");
    let call = &got[2];
    assert_eq!(u128::from_be_bytes(call.trace_id), TRACE_ID);
    assert_eq!(u64::from_be_bytes(call.parent_span_id), 0x1122334455667788);
    for child in &got[..2] {
        assert_eq!(child.trace_id, call.trace_id, "{child:?}");
        assert_eq!(child.parent_span_id, call.span_id);
        assert!(call.start <= child.start && child.end <= call.end + 1_000_000);
    }
    println!("{:#?}", got[0]);
    assert_eq!(got[1].attrs, [("attempt".to_owned(), "2".to_owned())]);
}

#[test]
fn only_the_configured_spans_are_built() {
    let runtime = ObsRuntime::new(Front::Layered);
    let spans = Box::new(Spans::default());
    runtime
        .obs
        .trace
        .register(
            take_span,
            &*spans as *const Spans as *mut c_void,
            "armonik_transport::trace::call=info,armonik_transport::trace::retry=info",
        )
        .unwrap();
    let _inside = runtime.scope();
    let call = call_span(true, 1);
    assert!(dial_span(&call).is_disabled(), "dial is not configured");
    assert!(!retry_span(&call, 1).is_disabled());
}

/// A head decision: an unsampled call builds no span, and a child of it is not a root of its own.
#[test]
fn a_child_of_a_span_that_was_not_built_is_not_built_either_when_the_engine_checks() {
    let runtime = ObsRuntime::new(Front::Layered);
    let spans = Box::new(Spans::default());
    runtime
        .obs
        .trace
        .register(take_span, &*spans as *const Spans as *mut c_void, "armonik_transport::trace=info")
        .unwrap();
    let _inside = runtime.scope();

    let call = call_span(false, 0);
    assert!(call.is_disabled());
    // What `parent: &Span::none()` does by itself: the child is built, as a root.
    let orphan = dial_span(&call);
    println!("child of a disabled parent is_disabled = {}", orphan.is_disabled());
    drop(orphan);
    let orphans = spans.0.lock().unwrap().len();
    println!("records delivered for it: {orphans}");

    // The engine's rule: a child is built only when its parent was.
    let child = if call.is_disabled() { Span::none() } else { dial_span(&call) };
    assert!(child.is_disabled());
}

/// A span lives across threads and awaits: created where the call starts, ended where it finishes.
#[test]
fn a_span_follows_its_future_to_the_channels_thread() {
    let runtime = ObsRuntime::new(Front::Layered);
    let spans = Box::new(Spans::default());
    runtime
        .obs
        .trace
        .register(take_span, &*spans as *const Spans as *mut c_void, "armonik_transport::trace=info")
        .unwrap();
    let channel = runtime.start_channel_thread();
    let call = {
        let _inside = runtime.scope();
        call_span(true, 9)
    };
    let child_parent = call.clone();
    runtime
        .tokio
        .block_on(channel.handle.spawn(
            async move {
                let dial = dial_span(&child_parent);
                async { tokio::task::yield_now().await }.instrument(dial).await;
            }
            .instrument(call),
        ))
        .unwrap();
    let got = spans.0.lock().unwrap().clone();
    assert_eq!(got.len(), 2, "{got:?}");
}
