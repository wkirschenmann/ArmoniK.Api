//! Owned byte buffers handed across the ABI, and the errors this crate reports through them.

use std::fmt;

use bytes::Bytes;

/// An owned buffer handed to the caller.
///
/// `ptr`/`len` are a read-only *view*; the actual owner of the allocation is `owner`, an opaque
/// handle the caller must pass back unchanged and never otherwise touch. Splitting it this way —
/// rather than treating `ptr` itself as the allocation to free — is what lets this type hand out a
/// [`bytes::Bytes`] this crate already owns (a decoded response message, most importantly) without
/// ever copying its content: `owner` is a boxed `Bytes`, `ptr`/`len` just describe what it derefs
/// to. Freeing then means dropping that box, which is correct regardless of what the `Bytes`
/// happens to wrap internally — a `Bytes` is not always backed by a plain allocation matching
/// `ptr`/`len` (it may be a shared, refcounted sub-slice), so reinterpreting `ptr` itself as
/// something to deallocate, the way a naive `Box<[u8]>` scheme would, is not an option here.
///
/// Every non-null (non-zero `owner`) `ak_bytes` must be released through exactly one call to
/// [`ak_bytes_free`]. The zeroed value (`owner`/`ptr` null, `len` 0) means "no data" and is always
/// safe to pass to [`ak_bytes_free`] as a no-op.
///
/// This is distinct from the plain `(ptr, len)` views the ABI uses for *input* buffers
/// ([`ak_bytes_in`]): those are borrowed from the caller and must never be freed by Rust. Only a
/// value that came out of this crate as an `ak_bytes` is ever appropriate to free through
/// [`ak_bytes_free`].
///
/// `Copy`, like any plain-old-data struct crossing a C ABI is on the C side — copying the
/// `(ptr, len, owner)` triple itself is harmless. What must never happen, on either side, is
/// passing more than one copy of the *same* originally-produced value to [`ak_bytes_free`].
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ak_bytes {
    /// Pointer to the first byte, or null for an empty/absent buffer. Valid to read from until
    /// this value is passed to [`ak_bytes_free`]; never write through it.
    pub ptr: *const u8,
    /// Number of bytes at `ptr`.
    pub len: usize,
    /// Opaque; pass back to [`ak_bytes_free`] unchanged, never dereference or otherwise inspect it.
    pub owner: *mut std::ffi::c_void,
}

impl ak_bytes {
    /// The zeroed value meaning "no data".
    pub(crate) const EMPTY: Self = Self {
        ptr: std::ptr::null(),
        len: 0,
        owner: std::ptr::null_mut(),
    };

    /// Take ownership of `data` into an `ak_bytes` the caller must eventually free, without
    /// copying its content.
    ///
    /// `Bytes::from` on a `Vec<u8>` or a `String`'s bytes reuses the existing allocation, so
    /// building `data` (from a freshly-assembled buffer, or from a `Bytes` this crate already held,
    /// e.g. a decoded response message) and handing it here moves ownership across the ABI in one
    /// step rather than copying and then leaking.
    pub(crate) fn from_bytes(data: impl Into<Bytes>) -> Self {
        let data = data.into();
        if data.is_empty() {
            return Self::EMPTY;
        }
        let ptr = data.as_ptr();
        let len = data.len();
        let owner = Box::into_raw(Box::new(data)).cast::<std::ffi::c_void>();
        Self { ptr, len, owner }
    }
}

/// Free an [`ak_bytes`] previously returned by this crate.
///
/// # Safety
///
/// `bytes` must be a value this crate returned, not yet freed. Passing a borrowed input buffer, a
/// value already freed, or a value with an `owner` that was not produced by this crate, is
/// undefined behaviour. The zeroed value is always safe to pass here.
#[no_mangle]
pub unsafe extern "C" fn ak_bytes_free(bytes: ak_bytes) {
    crate::guard::catch_unwind_void(|| {
        if bytes.owner.is_null() {
            return;
        }
        // SAFETY: per this function's contract, `bytes.owner` was produced by
        // `ak_bytes::from_bytes`, which always leaks exactly a `Box<Bytes>`. Dropping it runs
        // `Bytes`'s own destructor (a refcount decrement, freeing the backing allocation only once
        // the last reference goes away), rather than assuming `ptr`/`len` describe an allocation to
        // deallocate directly.
        drop(unsafe { Box::from_raw(bytes.owner.cast::<Bytes>()) });
    });
}

/// A borrowed input buffer: a view into memory the *caller* owns.
///
/// Never freed by this crate. A null `ptr` or zero `len` means "empty" or "absent", matching the
/// convention `armonik_transport::ClientConfigArgs` uses for its string fields.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ak_bytes_in {
    /// Pointer to the first byte, or null for an empty/absent buffer.
    pub ptr: *const u8,
    /// Number of bytes at `ptr`.
    pub len: usize,
}

impl ak_bytes_in {
    /// Borrow the bytes as a slice.
    ///
    /// # Safety
    ///
    /// `ptr` must be valid for `len` bytes for the duration of the borrow, or both must be zero.
    pub(crate) unsafe fn as_slice<'a>(&self) -> &'a [u8] {
        if self.ptr.is_null() || self.len == 0 {
            &[]
        } else {
            // SAFETY: forwarded from the caller of this function's caller.
            unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
        }
    }
}

/// Errors reported by this crate's own logic, as opposed to a gRPC status coming back from the
/// server.
///
/// Kept separate from [`armonik_transport::ConfigError`] rather than trying to reuse its variants:
/// `ConfigError` is `#[non_exhaustive]` and this crate only ever constructs errors, so there is
/// nothing to gain by fighting that boundary.
#[derive(Debug)]
pub(crate) enum FfiError {
    NullArgument(&'static str),
    InvalidUtf8,
    Config(armonik_transport::ConfigError),
    MismatchedIdentity,
    InvalidCertPem(String),
    InvalidKeyPem(String),
    InvalidCaCertPem(String),
    Connection(armonik_transport::ConnectionError),
    InvalidHandle,
    InvalidState(&'static str),
    EventCreation(String),
}

impl fmt::Display for FfiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NullArgument(name) => write!(f, "`{name}` must not be null"),
            Self::InvalidUtf8 => write!(f, "a buffer that was expected to be UTF-8 was not"),
            Self::Config(source) => write!(f, "{source}"),
            Self::MismatchedIdentity => write!(
                f,
                "`cert_pem` and `key_pem` must either both be empty or both be set"
            ),
            Self::InvalidCertPem(source) => write!(f, "invalid `cert_pem`: {source}"),
            Self::InvalidKeyPem(source) => write!(f, "invalid `key_pem`: {source}"),
            Self::InvalidCaCertPem(source) => write!(f, "invalid `ca_cert`: {source}"),
            Self::Connection(source) => write!(f, "{source}"),
            Self::InvalidHandle => write!(f, "the handle is invalid or has already been freed"),
            Self::InvalidState(reason) => write!(f, "{reason}"),
            Self::EventCreation(source) => {
                write!(f, "could not create the wait event for the call: {source}")
            }
        }
    }
}

impl FfiError {
    /// The negative status code this error is reported as.
    pub(crate) fn status(&self) -> i32 {
        match self {
            Self::NullArgument(_) => crate::status::NULL_ARGUMENT,
            Self::InvalidUtf8 => crate::status::INVALID_UTF8,
            Self::Config(_)
            | Self::MismatchedIdentity
            | Self::InvalidCertPem(_)
            | Self::InvalidKeyPem(_)
            | Self::InvalidCaCertPem(_) => crate::status::INVALID_CONFIG,
            Self::Connection(_) => crate::status::CONNECTION_FAILED,
            Self::InvalidHandle => crate::status::INVALID_HANDLE,
            Self::InvalidState(_) => crate::status::INVALID_STATE,
            Self::EventCreation(_) => crate::status::INTERNAL,
        }
    }

    /// Render this error into the `(status, ak_bytes)` pair every fallible entry point returns.
    pub(crate) fn into_ffi_result(self, out_err: *mut ak_bytes) -> i32 {
        let status = self.status();
        if !out_err.is_null() {
            // SAFETY: `out_err` is documented as writable by every function that takes it.
            unsafe { *out_err = ak_bytes::from_bytes(self.to_string()) };
        }
        status
    }
}

impl From<armonik_transport::ConfigError> for FfiError {
    fn from(source: armonik_transport::ConfigError) -> Self {
        Self::Config(source)
    }
}

impl From<armonik_transport::ConnectionError> for FfiError {
    fn from(source: armonik_transport::ConnectionError) -> Self {
        Self::Connection(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read back what the caller would see, then free it exactly once.
    ///
    /// # Safety
    ///
    /// `bytes` must be a value `from_bytes` produced and that has not been freed.
    unsafe fn read_and_free(bytes: ak_bytes) -> Vec<u8> {
        let seen = if bytes.len == 0 {
            Vec::new()
        } else {
            // SAFETY: `ptr`/`len` are documented as readable until `ak_bytes_free`.
            unsafe { std::slice::from_raw_parts(bytes.ptr, bytes.len) }.to_vec()
        };
        // SAFETY: forwarded from this function's contract.
        unsafe { ak_bytes_free(bytes) };
        seen
    }

    #[test]
    fn an_owned_buffer_is_readable_through_the_view_it_hands_out() {
        let bytes = ak_bytes::from_bytes(b"hello".to_vec());

        assert!(!bytes.owner.is_null(), "a non-empty buffer has an owner");
        assert_eq!(bytes.len, 5);
        // SAFETY: just produced above, freed exactly once here.
        assert_eq!(unsafe { read_and_free(bytes) }, b"hello");
    }

    #[test]
    fn a_sub_slice_is_freed_through_its_owner_rather_than_its_pointer() {
        // The case the split between `ptr` and `owner` exists for. A `Bytes` may be a refcounted view
        // into the middle of a larger allocation, so `ptr` is not something that can be deallocated:
        // an implementation that treated the view as the allocation would corrupt the heap here, which
        // is exactly what this test asks Miri to check.
        let whole = Bytes::from_static(b"0123456789");
        let middle = whole.slice(3..7);

        let bytes = ak_bytes::from_bytes(middle);
        assert_eq!(bytes.len, 4);
        // SAFETY: produced just above, freed exactly once.
        assert_eq!(unsafe { read_and_free(bytes) }, b"3456");
    }

    #[test]
    fn an_empty_buffer_needs_no_owner_and_frees_as_a_no_op() {
        let bytes = ak_bytes::from_bytes(Vec::new());

        assert!(bytes.ptr.is_null());
        assert_eq!(bytes.len, 0);
        assert!(
            bytes.owner.is_null(),
            "nothing was allocated, so there is nothing for the caller to free"
        );
        // SAFETY: the zeroed value is documented as always safe to free, more than once.
        unsafe {
            ak_bytes_free(bytes);
            ak_bytes_free(bytes);
        }
    }

    #[test]
    fn an_error_renders_its_message_and_status_together() {
        let mut out = ak_bytes::EMPTY;
        let status = FfiError::NullArgument("out").into_ffi_result(std::ptr::addr_of_mut!(out));

        assert_eq!(status, crate::status::NULL_ARGUMENT);
        // SAFETY: written by `into_ffi_result` just above, freed exactly once.
        assert_eq!(unsafe { read_and_free(out) }, b"`out` must not be null");
    }

    #[test]
    fn an_error_reported_without_a_message_buffer_still_returns_its_status() {
        // `out_err` is optional on every entry point that takes it, so a caller that does not want the
        // message must not be a special case for the error path.
        let status = FfiError::InvalidHandle.into_ffi_result(std::ptr::null_mut());
        assert_eq!(status, crate::status::INVALID_HANDLE);
    }

    #[test]
    fn a_borrowed_input_buffer_reads_as_a_slice_and_treats_null_as_empty() {
        let data = b"borrowed";
        let view = ak_bytes_in {
            ptr: data.as_ptr(),
            len: data.len(),
        };
        // SAFETY: `data` outlives the borrow.
        assert_eq!(unsafe { view.as_slice() }, data);

        let absent = ak_bytes_in {
            ptr: std::ptr::null(),
            len: 0,
        };
        // SAFETY: the null/zero case is explicitly allowed.
        assert!(unsafe { absent.as_slice() }.is_empty());
    }
}
