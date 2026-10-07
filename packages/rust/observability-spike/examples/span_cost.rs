//! Point 4: what a span costs, disabled and enabled, and what carrying a dispatcher per poll costs.
//!
//! span_cost <scenario>
//!   none        no dispatcher in the process
//!   off         one runtime, no trace callback
//!   off2        one runtime without a trace callback, another with one (the callsite is `sometimes`)
//!   flag        as `off`, with the engine checking a runtime flag before it builds the span
//!   on          one runtime with a trace callback and the span configured
//!   on3         as `on`, a span with three attributes and a child
//!   poll        a future polled 1M times: bare, and under `with_subscriber`

use std::ffi::c_void;
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::task::{Context, Poll, Waker};
use std::time::Instant;

use observability_spike::runtime::{Front, ObsRuntime};
use observability_spike::trace::TraceRecord;
use tracing::instrument::WithSubscriber;
use tracing::{info_span, Span};

static DELIVERED: AtomicU64 = AtomicU64::new(0);

unsafe extern "C" fn count_span(_: *mut c_void, _: *const TraceRecord) {
    DELIVERED.fetch_add(1, Ordering::Relaxed);
}

static FLAG: AtomicBool = AtomicBool::new(false);

#[inline(never)]
fn make_span(i: u64) -> Span {
    info_span!(target: "armonik_transport::trace::attempt", "attempt", attempt = i)
}

#[inline(never)]
fn make_span_flagged(i: u64) -> Span {
    if FLAG.load(Ordering::Relaxed) {
        info_span!(target: "armonik_transport::trace::attempt", "attempt", attempt = i)
    } else {
        Span::none()
    }
}

#[inline(never)]
fn lifecycle(i: u64) {
    let span = make_span(i);
    let _entered = span.enter();
    black_box(i);
}

#[inline(never)]
fn lifecycle3(i: u64) {
    let call = info_span!(target: "armonik_transport::trace::attempt", "call", method = "/x/Y", attempt = i, endpoint = "http://127.0.0.1:5001");
    let child = info_span!(target: "armonik_transport::trace::attempt", parent: &call, "dial", attempt = i);
    let _entered = child.enter();
    black_box(i);
}

fn time(label: &str, n: u64, mut f: impl FnMut(u64)) {
    let mut samples = Vec::new();
    for _ in 0..7 {
        let start = Instant::now();
        for i in 0..n {
            f(black_box(i));
        }
        samples.push(start.elapsed().as_secs_f64() * 1e9 / n as f64);
    }
    samples.sort_by(f64::total_cmp);
    println!("{label:58} min {:8.2}  median {:8.2}  max {:8.2} ns", samples[0], samples[3], samples[6]);
}

struct Yielding(u32);
impl std::future::Future for Yielding {
    type Output = ();
    fn poll(mut self: std::pin::Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
        if self.0 == 0 {
            Poll::Ready(())
        } else {
            self.0 -= 1;
            Poll::Pending
        }
    }
}

fn main() {
    let scenario = std::env::args().nth(1).unwrap_or_else(|| "off".into());
    let n = 5_000_000;
    let register = |runtime: &ObsRuntime| {
        runtime
            .obs
            .trace
            .register(count_span, std::ptr::null_mut(), "armonik_transport::trace=info")
            .unwrap();
    };
    let runtime = ObsRuntime::new(Front::Layered);
    let other = ObsRuntime::new(Front::Layered);
    let _guard = match scenario.as_str() {
        "none" => None,
        "off" | "flag" | "poll" => Some(runtime.scope()),
        "off2" => {
            register(&other);
            Some(runtime.scope())
        }
        "on" | "on3" => {
            register(&runtime);
            FLAG.store(true, Ordering::Relaxed);
            Some(runtime.scope())
        }
        other => panic!("{other}"),
    };
    match scenario.as_str() {
        "none" | "off" | "off2" => time(&format!("{scenario}: info_span! built and dropped"), n, |i| drop(make_span(i))),
        "flag" => time("flag: flag-checked, flag off", n, |i| drop(make_span_flagged(i))),
        "on" => {
            time("on: span built, entered, exited, closed -> callback", n / 5, lifecycle);
            println!("delivered {}", DELIVERED.load(Ordering::Relaxed));
        }
        "on3" => {
            time("on3: two spans, 4 attributes, child -> 2 callbacks", n / 5, lifecycle3);
            println!("delivered {}", DELIVERED.load(Ordering::Relaxed));
        }
        "poll" => {
            let poll_many = |dispatch: Option<tracing::Dispatch>| {
                let mut samples = Vec::new();
                for _ in 0..7 {
                    let polls = 2_000_000u32;
                    let start = Instant::now();
                    let waker = Waker::noop();
                    let mut cx = Context::from_waker(waker);
                    match &dispatch {
                        None => {
                            let mut f = Box::pin(Yielding(polls));
                            while f.as_mut().poll(&mut cx).is_pending() {}
                        }
                        Some(d) => {
                            let mut f = Box::pin(Yielding(polls).with_subscriber(d.clone()));
                            while f.as_mut().poll(&mut cx).is_pending() {}
                        }
                    }
                    samples.push(start.elapsed().as_secs_f64() * 1e9 / polls as f64);
                }
                samples.sort_by(f64::total_cmp);
                (samples[0], samples[3], samples[6])
            };
            let bare = poll_many(None);
            let wrapped = poll_many(Some(runtime.dispatch.clone()));
            println!("poll bare:             min {:6.2} median {:6.2} max {:6.2} ns/poll", bare.0, bare.1, bare.2);
            println!("poll with_subscriber:  min {:6.2} median {:6.2} max {:6.2} ns/poll", wrapped.0, wrapped.1, wrapped.2);
        }
        _ => unreachable!(),
    }
}
