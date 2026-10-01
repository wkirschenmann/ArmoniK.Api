//! Why an entry point refused, and how that reaches the host's `ak_error`.
//!
//! The message is rendered only when the host passed an `ak_error` to write it into: a host
//! that passes NULL is answered by the status alone and pays no allocation for a message it did
//! not ask for. A constant message crosses as itself, unowned.

use std::borrow::Cow;
use std::error::Error;
use std::ffi::c_void;
use std::fmt;

use armonik_transport::grpc::{ChannelError, GrpcChannelConfigError};

use crate::abi::{ak_bytes, ak_error, ak_error_kind, ak_status};
use crate::config::ConfigRefusal;
use crate::tagged::take_tagged;

const DETAIL_TAG: u64 = 0x414b_5f45_5252_4f00;

/// A refusal: the status every host reads, and the family and cause a host that asked reads too.
pub(crate) struct Refusal {
    status: ak_status,
    kind: ak_error_kind,
    cause: Cause,
}

enum Cause {
    Fixed(&'static str),
    Config(ConfigRefusal),
    Channel(GrpcChannelConfigError),
    Call(ChannelError),
    /// A message whose rendering panics, which no refusal of this library builds.
    #[cfg(test)]
    Panics,
}

impl Refusal {
    pub(crate) const fn fixed(status: ak_status, kind: ak_error_kind, why: &'static str) -> Self {
        Self {
            status,
            kind,
            cause: Cause::Fixed(why),
        }
    }

    pub(crate) fn config(refused: ConfigRefusal) -> Self {
        Self {
            status: ak_status::AK_STATUS_INVALID_ARG,
            kind: ak_error_kind::AK_ERROR_CONFIG,
            cause: Cause::Config(refused),
        }
    }

    /// What the engine refused when the channel was built. Building one opens no socket, so the
    /// refusal is of its configuration, the transport's included.
    pub(crate) fn channel(refused: GrpcChannelConfigError) -> Self {
        Self {
            status: ak_status::AK_STATUS_INVALID_ARG,
            kind: ak_error_kind::AK_ERROR_CONFIG,
            cause: Cause::Channel(refused),
        }
    }

    /// What the engine refused when a call was started on a channel. Starting one opens no
    /// stream, so what the network does reaches the host as the call's status event instead.
    pub(crate) fn call(refused: ChannelError) -> Self {
        let (status, kind) = match &refused {
            ChannelError::Closed => (
                ak_status::AK_STATUS_INVALID_STATE,
                ak_error_kind::AK_ERROR_USAGE,
            ),
            ChannelError::InvalidMethod { .. } | ChannelError::InvalidMetadata { .. } => (
                ak_status::AK_STATUS_INVALID_ARG,
                ak_error_kind::AK_ERROR_USAGE,
            ),
            _ => (ak_status::AK_STATUS_INTERNAL, ak_error_kind::AK_ERROR_NONE),
        };
        Self {
            status,
            kind,
            cause: Cause::Call(refused),
        }
    }

    /// The message, rendered: a constant as itself, anything else flattened.
    fn detail(&self) -> Cow<'static, str> {
        match &self.cause {
            Cause::Fixed(why) => Cow::Borrowed(why),
            Cause::Config(refused) => Cow::Owned(flattened(refused)),
            Cause::Channel(refused) => Cow::Owned(flattened(refused)),
            Cause::Call(refused) => Cow::Owned(flattened(refused)),
            #[cfg(test)]
            Cause::Panics => panic!("a Display that panics"),
        }
    }
}

/// The refusal a status alone makes, for the places that know nothing more.
impl From<ak_status> for Refusal {
    fn from(status: ak_status) -> Self {
        use ak_error_kind::{AK_ERROR_NONE as NONE, AK_ERROR_USAGE as USAGE};
        use ak_status::*;
        let (kind, why) = match status {
            AK_STATUS_OK => (NONE, "the call succeeded"),
            AK_STATUS_HANDLE_STALE => (USAGE, "the handle names nothing this library holds"),
            AK_STATUS_SLOT_BUSY => (
                NONE,
                "this call's send window is full; its next WRITE_DONE frees a slot",
            ),
            AK_STATUS_INVALID_ARG => (
                USAGE,
                "an argument this library does not admit: a null pointer, a value out of range, or a struct of a size it does not read",
            ),
            AK_STATUS_INTERNAL => (NONE, "a fault this library cannot attribute"),
            AK_STATUS_BUDGET_BUSY => (NONE, "the runtime-wide byte ceiling is reached"),
            AK_STATUS_INVALID_STATE => (USAGE, "the object the handle names refuses this in its current state"),
            AK_STATUS_MESSAGE_TOO_LARGE => {
                (NONE, "the length exceeds the runtime-wide ceiling itself")
            }
        };
        Self::fixed(status, kind, why)
    }
}

/// The cause chain in one line, without the source locations snafu renders.
///
/// A source is appended only when the text so far does not already end with it, since a variant
/// that displays as `{source}` would otherwise say everything twice.
fn flattened(error: &dyn Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let said = cause.to_string();
        if !text.ends_with(&said) {
            text.push_str(": ");
            text.push_str(&said);
        }
        source = cause.source();
    }
    without_locations(&text)
}

/// The text with every ` [path.rs:line:column]` taken out: a source location is the tracing
/// record's, and requirement 11.4 keeps it out of what crosses the ABI.
fn without_locations(text: &str) -> String {
    let mut kept = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find(" [") {
        let inside = &rest[open + 2..];
        let Some(close) = inside.find(']') else {
            break;
        };
        kept.push_str(&rest[..open]);
        if !is_location(&inside[..close]) {
            kept.push_str(&rest[open..open + 2 + close + 1]);
        }
        rest = &inside[close + 1..];
    }
    kept.push_str(rest);
    kept
}

/// `path.rs:line:column`, as snafu's `Location` displays.
fn is_location(text: &str) -> bool {
    let mut parts = text.rsplitn(3, ':');
    let (Some(column), Some(line), Some(path)) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let number = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    number(column) && number(line) && path.ends_with(".rs") && !path.contains(char::is_whitespace)
}

const RENDER_PANICKED: &str = "this library panicked rendering the message";

fn unowned(text: &'static str) -> ak_bytes {
    ak_bytes {
        ptr: text.as_ptr(),
        len: text.len(),
        owner: std::ptr::null_mut(),
    }
}

/// An owned message, handed to the host behind its tag.
#[repr(C)]
struct Detail {
    tag: u64,
    text: String,
}

/// The status of `answered`, and its refusal written into `*out_error` when the host passed one.
///
/// Nothing is rendered when `out_error` is null, and nothing is written when the call succeeded.
///
/// # Safety
///
/// `out_error` must be null or writable for an `ak_error`.
pub(crate) unsafe fn answer(out_error: *mut ak_error, answered: Result<(), Refusal>) -> ak_status {
    let refused = match answered {
        Ok(()) => return ak_status::AK_STATUS_OK,
        Err(refused) => refused,
    };
    if !out_error.is_null() {
        // A Display that panics costs the host its message, not its process.
        let detail = crate::guard_with(unowned(RENDER_PANICKED), || match refused.detail() {
            Cow::Borrowed(text) => unowned(text),
            Cow::Owned(text) => {
                let detail = Box::new(Detail {
                    tag: DETAIL_TAG,
                    text,
                });
                ak_bytes {
                    ptr: detail.text.as_ptr(),
                    len: detail.text.len(),
                    owner: Box::into_raw(detail) as *mut c_void,
                }
            }
        });
        unsafe {
            out_error.write(ak_error {
                kind: refused.kind,
                detail,
            })
        };
    }
    refused.status
}

/// Frees what `answer` allocated for a detail; a constant one, NULL owner, is left alone.
///
/// # Safety
///
/// `owner` must be null, or a detail's owner this library wrote and the host has not released.
pub(crate) unsafe fn release(owner: *mut c_void) {
    drop(unsafe { take_tagged::<Detail>(owner, DETAIL_TAG) });
}

impl fmt::Debug for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Refusal")
            .field("status", &self.status)
            .field("kind", &self.kind)
            .field("detail", &self.detail())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_location_is_taken_out_of_the_message() {
        assert_eq!(
            without_locations("Invalid TLS configuration [src/config.rs:12:5]: bad PEM"),
            "Invalid TLS configuration: bad PEM"
        );
        assert_eq!(
            without_locations(r"Could not read [C:\a b\src\x.rs:1:2]"),
            r"Could not read [C:\a b\src\x.rs:1:2]",
            "a path with a space is not what snafu renders, so it stays"
        );
    }

    #[test]
    fn brackets_that_are_not_a_location_stay() {
        for text in [
            "a list [1, 2] of numbers",
            "an address [::1]:5000",
            "a lone [ bracket",
            "a file [notes.txt:1:2]",
        ] {
            assert_eq!(without_locations(text), text);
        }
    }

    #[derive(Debug)]
    struct Outer(Inner);
    #[derive(Debug)]
    struct Inner;

    impl fmt::Display for Outer {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "outer: {}", self.0)
        }
    }
    impl fmt::Display for Inner {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("inner [src/inner.rs:3:9]")
        }
    }
    impl Error for Outer {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            Some(&self.0)
        }
    }
    impl Error for Inner {}

    #[test]
    fn a_cause_the_text_already_ends_with_is_not_said_twice() {
        assert_eq!(flattened(&Outer(Inner)), "outer: inner");
    }

    #[test]
    fn a_message_that_panics_still_answers() {
        let refused = Refusal {
            status: ak_status::AK_STATUS_INVALID_ARG,
            kind: ak_error_kind::AK_ERROR_CONFIG,
            cause: Cause::Panics,
        };
        let mut error = std::mem::MaybeUninit::<ak_error>::uninit();
        let status = unsafe { answer(error.as_mut_ptr(), Err(refused)) };
        assert_eq!(status, ak_status::AK_STATUS_INVALID_ARG);
        let error = unsafe { error.assume_init() };
        assert_eq!(error.kind, ak_error_kind::AK_ERROR_CONFIG);
        let said = unsafe { std::slice::from_raw_parts(error.detail.ptr, error.detail.len) };
        assert_eq!(said, RENDER_PANICKED.as_bytes());
        assert!(error.detail.owner.is_null());
    }
}
