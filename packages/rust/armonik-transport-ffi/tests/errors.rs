//! What a refusal writes into the host's `ak_error`, and what it costs a host that passes none.
//!
//! The allocations are counted per thread, so a count is the calling thread's alone and the
//! runtime's workers do not enter it.

// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::mem::MaybeUninit;
use std::ptr::null_mut;

use armonik_transport_ffi::*;
use support::host::*;

struct Counting;

thread_local! {
    static MADE: Cell<usize> = const { Cell::new(0) };
    static LIVE: Cell<isize> = const { Cell::new(0) };
}

/// A thread being torn down has no counters left, and its allocations are no test's.
fn count(made: usize, live: isize) {
    let _ = MADE.try_with(|cell| cell.set(cell.get() + made));
    let _ = LIVE.try_with(|cell| cell.set(cell.get() + live));
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(1, 1);
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count(1, 1);
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count(1, 0);
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        count(0, -1);
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

/// What `body` allocated on this thread, and how many of those allocations outlive it.
#[derive(Debug)]
struct Counts {
    made: usize,
    live: isize,
}

fn counted<T>(body: impl FnOnce() -> T) -> (T, Counts) {
    let (made, live) = (MADE.with(Cell::get), LIVE.with(Cell::get));
    let answered = body();
    let counts = Counts {
        made: MADE.with(Cell::get) - made,
        live: LIVE.with(Cell::get) - live,
    };
    (answered, counts)
}

/// Refused over its value's type, which serde's own message does not name the key of.
const REFUSED: &str = r#"{"Grpc":{"Host":{"Receive":{"Credits":"2"}}}}"#;

fn create_channel(host: &Host, json: &str, out_error: *mut ak_error) -> ak_status {
    let endpoint = "http://localhost:1";
    let mut channel = AK_HANDLE_NONE;
    unsafe {
        ak_channel_create(
            host.runtime,
            ak_bytes_in {
                ptr: endpoint.as_ptr(),
                len: endpoint.len(),
            },
            ak_bytes_in {
                ptr: json.as_ptr(),
                len: json.len(),
            },
            &mut channel,
            out_error,
        )
    }
}

fn text(detail: &ak_bytes) -> &str {
    std::str::from_utf8(unsafe { std::slice::from_raw_parts(detail.ptr, detail.len) }).unwrap()
}

#[test]
fn a_host_that_passes_no_error_pays_for_no_message() {
    let host = Host::start();
    // The first refusal of the process sets up what later ones reuse, which neither count is of.
    create_channel(&host, REFUSED, null_mut());

    let (status, without) = counted(|| create_channel(&host, REFUSED, null_mut()));
    assert_eq!(status, ak_status::AK_STATUS_INVALID_ARG);
    assert_eq!(
        without.live, 0,
        "a refusal without an ak_error leaves nothing allocated"
    );

    let mut error = MaybeUninit::<ak_error>::uninit();
    let (status, with) = counted(|| create_channel(&host, REFUSED, error.as_mut_ptr()));
    assert_eq!(status, ak_status::AK_STATUS_INVALID_ARG);
    let error = unsafe { error.assume_init() };
    // Each allocation the detail still holds is one the call without an ak_error never made.
    assert!(
        with.made >= without.made + with.live as usize,
        "the message is rendered without an ak_error to put it in: {without:?} against {with:?}"
    );

    let ((), released) = counted(|| unsafe { ak_error_release(error.detail) });
    assert_eq!(
        with.live + released.live,
        0,
        "ak_error_release frees what the refusal allocated for its detail"
    );
    host.stop();
}

#[test]
fn a_refused_document_names_its_key() {
    let host = Host::start();
    let mut error = MaybeUninit::<ak_error>::uninit();
    let status = create_channel(&host, REFUSED, error.as_mut_ptr());
    assert_eq!(status, ak_status::AK_STATUS_INVALID_ARG);

    let error = unsafe { error.assume_init() };
    assert_eq!(error.kind, ak_error_kind::AK_ERROR_CONFIG);
    assert!(!error.detail.owner.is_null(), "a rendered message is owned");
    let said = text(&error.detail);
    assert!(said.contains("Grpc.Host.Receive.Credits"), "{said}");
    assert!(
        !said.contains(".rs:"),
        "a source location crosses the ABI: {said}"
    );

    unsafe { ak_error_release(error.detail) };
    host.stop();
}

#[test]
fn a_constant_message_costs_nothing_and_is_not_owned() {
    let stale = AK_HANDLE_NONE;

    let (status, without) = counted(|| unsafe { ak_call_cancel(stale, null_mut()) });
    assert_eq!(status, ak_status::AK_STATUS_HANDLE_STALE);

    let mut error = MaybeUninit::<ak_error>::uninit();
    let (status, with) = counted(|| unsafe { ak_call_cancel(stale, error.as_mut_ptr()) });
    assert_eq!(status, ak_status::AK_STATUS_HANDLE_STALE);
    assert_eq!((without.made, with.made), (0, 0));

    let error = unsafe { error.assume_init() };
    assert_eq!(error.kind, ak_error_kind::AK_ERROR_USAGE);
    assert!(error.detail.owner.is_null());
    assert!(!text(&error.detail).is_empty());
    // Releasing an unowned detail is allowed, and does nothing.
    unsafe { ak_error_release(error.detail) };
}

#[test]
fn a_call_that_succeeds_leaves_the_error_alone() {
    let host = Host::start();
    let untouched = ak_error {
        kind: ak_error_kind::AK_ERROR_TIMEOUT,
        detail: ak_bytes {
            ptr: c"sentinel".as_ptr().cast(),
            len: 8,
            owner: null_mut(),
        },
    };
    let mut error = untouched;
    let mut usage = MaybeUninit::<ak_memory_usage>::uninit();
    let status = unsafe { ak_runtime_memory_usage(host.runtime, usage.as_mut_ptr(), &mut error) };
    assert_eq!(status, ak_status::AK_STATUS_OK);
    assert_eq!(error.kind, untouched.kind);
    assert_eq!(error.detail.ptr, untouched.detail.ptr);
    host.stop();
}
