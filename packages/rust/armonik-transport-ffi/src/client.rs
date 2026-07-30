//! The connected client handle.

use armonik_transport::reexports::tonic::client::Grpc;
use armonik_transport::reexports::tonic::transport::Channel;
use armonik_transport::RetryPolicy;

use std::sync::OnceLock;

use crate::config::Certificates;
use crate::error::{ak_bytes, ak_bytes_in, FfiError};
use crate::handle::LiveSet;

static LIVE: OnceLock<LiveSet> = OnceLock::new();

fn live() -> &'static LiveSet {
    LIVE.get_or_init(LiveSet::new)
}

/// An opaque, connected client.
///
/// Obtained from [`ak_client_create`], released with [`ak_client_free`]. Every operation on it
/// takes a shared reference recovered from the raw pointer, and the underlying `tonic` channel is
/// itself safe to use from multiple calls concurrently, so one client can back many concurrent
/// [`crate::call::ak_call`]s.
pub struct ak_client {
    pub(crate) grpc: Grpc<Channel>,
    pub(crate) retry: Option<RetryPolicy>,
}

impl ak_client {
    fn connect(mut config: armonik_transport::ClientConfig) -> Result<Self, FfiError> {
        // Moved out, not cloned: `connect` never reads `retry` (it is purely a concern of this
        // crate's own retry loop in `call.rs`), so taking it leaves nothing behind that is needed.
        let retry = config.retry.take();
        let channel = crate::runtime::handle().block_on(armonik_transport::connect(config))?;
        Ok(Self {
            grpc: Grpc::new(channel),
            retry,
        })
    }
}

/// Whether `ptr` is currently a live client handle.
///
/// Used by [`crate::call`] to reject a call started against a client that has already been freed,
/// without dereferencing it.
pub(crate) fn is_live(ptr: *const ak_client) -> bool {
    live().contains(ptr)
}

/// Create a client and connect it eagerly.
///
/// `options` is a key/value blob naming the options to set: the format is documented in the
/// generated header, and the names are exactly
/// [`armonik_transport::ClientConfigArgs::OPTION_NAMES`]. Absent options take their default, so any
/// option may be omitted — though an `Endpoint` is required for the client to have anywhere to go.
/// `cert_pem`/`key_pem`/`ca_cert` carry PEM bytes rather than paths, and `cert_pem`/`key_pem` must
/// either both be present or both be absent.
///
/// On success, `*out` receives a handle that must later be released with [`ak_client_free`]. On
/// failure, `*out_err` (when non-null) receives the error message as an owned [`ak_bytes`], to be
/// released with [`crate::error::ak_bytes_free`].
///
/// # Safety
///
/// `options`, if non-null, must be valid for `options_len` bytes, and each of `cert_pem`, `key_pem`
/// and `ca_cert` must satisfy [`ak_bytes_in`]'s contract, for the duration of this call. `out` must
/// be non-null and writable. `out_err`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn ak_client_create(
    options: *const u8,
    options_len: usize,
    cert_pem: ak_bytes_in,
    key_pem: ak_bytes_in,
    ca_cert: ak_bytes_in,
    out: *mut *mut ak_client,
    out_err: *mut ak_bytes,
) -> i32 {
    crate::guard::catch_unwind_status(out_err, || {
        if out.is_null() {
            return FfiError::NullArgument("out").into_ffi_result(out_err);
        }

        let certificates = Certificates {
            cert_pem,
            key_pem,
            ca_cert_pem: ca_cert,
        };

        // SAFETY: forwarded from this function's own contract.
        let config = match unsafe { crate::config::build(options, options_len, certificates) } {
            Ok(config) => config,
            Err(error) => return error.into_ffi_result(out_err),
        };

        match ak_client::connect(config) {
            Ok(client) => {
                let ptr = Box::into_raw(Box::new(client));
                live().insert(ptr);
                // SAFETY: `out` was checked non-null above.
                unsafe { *out = ptr };
                crate::status::OK
            }
            Err(error) => error.into_ffi_result(out_err),
        }
    })
}

/// Release a client handle.
///
/// [`crate::call::ak_call_start`] clones the underlying channel and retry policy out of the client
/// while it is still known to be live, so a call already in flight keeps working unaffected after
/// its client is freed: the client handle is only ever needed to *start* new calls.
///
/// # Safety
///
/// `client` must be a value returned by [`ak_client_create`] that has not already been freed, or
/// null (a no-op).
#[no_mangle]
pub unsafe extern "C" fn ak_client_free(client: *mut ak_client) {
    crate::guard::catch_unwind_void(|| {
        if client.is_null() {
            return;
        }
        if !live().remove(client) {
            // Already freed, or never a valid handle: do not touch the memory.
            return;
        }
        // SAFETY: `live().remove` just returned `true`, so this address was produced by
        // `ak_client_create` and has not been freed since.
        drop(unsafe { Box::from_raw(client) });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode an options blob, the way the .NET side will.
    fn options(pairs: &[(&str, &str)]) -> Vec<u8> {
        let mut blob = (pairs.len() as u32).to_ne_bytes().to_vec();
        for (key, value) in pairs {
            blob.extend_from_slice(&(key.len() as u32).to_ne_bytes());
            blob.extend_from_slice(key.as_bytes());
            blob.extend_from_slice(&(value.len() as u32).to_ne_bytes());
            blob.extend_from_slice(value.as_bytes());
        }
        blob
    }

    fn empty_in() -> ak_bytes_in {
        ak_bytes_in {
            ptr: std::ptr::null(),
            len: 0,
        }
    }

    /// Options for an endpoint, with a short connect timeout so the unreachable case cannot hang.
    fn options_for(endpoint: &str) -> Vec<u8> {
        options(&[("Endpoint", endpoint), ("ConnectTimeout", "2s")])
    }

    /// Call `ak_client_create` with an options blob and no certificates.
    fn create(blob: &[u8]) -> (i32, *mut ak_client, ak_bytes) {
        let mut out: *mut ak_client = std::ptr::null_mut();
        let mut err = ak_bytes::EMPTY;
        // SAFETY: `blob` is live across the call, the certificate views are all empty, and both
        // out-parameters point at live locals.
        let status = unsafe {
            ak_client_create(
                blob.as_ptr(),
                blob.len(),
                empty_in(),
                empty_in(),
                empty_in(),
                std::ptr::addr_of_mut!(out),
                std::ptr::addr_of_mut!(err),
            )
        };
        (status, out, err)
    }

    /// Serve a stub gRPC service on an ephemeral loopback port, from the same runtime this crate
    /// uses, so it is reachable from a synchronous `ak_client_create`.
    fn spawn_server() -> std::net::SocketAddr {
        use std::sync::Arc;

        use armonik::server::{RequestContext, VersionsServiceExt};
        use armonik::versions;

        #[derive(Debug, Clone, Default)]
        struct Service;

        impl armonik::server::VersionsService for Service {
            async fn list(
                self: Arc<Self>,
                _request: versions::list::Request,
                _context: RequestContext,
            ) -> Result<versions::list::Response, armonik::reexports::tonic::Status> {
                Ok(versions::list::Response::default())
            }
        }

        crate::runtime::handle().block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind");
            let address = listener.local_addr().expect("address");
            tokio::spawn(async move {
                let incoming =
                    armonik::reexports::tokio_stream::wrappers::TcpListenerStream::new(listener);
                armonik::reexports::tonic::transport::Server::builder()
                    .add_service(Service.versions_server())
                    .serve_with_incoming(incoming)
                    .await
                    .expect("serve");
            });
            address
        })
    }

    #[test]
    fn a_reachable_endpoint_creates_a_client() {
        let address = spawn_server();
        let (status, client, err) = create(&options_for(&format!("http://{address}")));

        assert_eq!(status, crate::status::OK);
        assert!(!client.is_null());
        assert!(is_live(client));
        // SAFETY: `err` is the zeroed value on success, which is a documented no-op.
        unsafe { crate::error::ak_bytes_free(err) };

        // SAFETY: `client` came from `ak_client_create` and has not been freed.
        unsafe { ak_client_free(client) };
        assert!(!is_live(client));
    }

    #[test]
    fn an_unreachable_endpoint_is_reported_rather_than_hanging() {
        // Bind then drop, so nothing is listening there; the connect timeout bounds the wait.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        drop(listener);

        let (status, client, err) = create(&options_for(&format!("http://{address}")));

        assert_eq!(status, crate::status::CONNECTION_FAILED);
        assert!(client.is_null());
        assert!(!err.owner.is_null(), "an error message should be produced");
        // SAFETY: produced by the failed call above, freed exactly once.
        unsafe { crate::error::ak_bytes_free(err) };
    }

    #[test]
    fn an_invalid_option_never_reaches_the_connection_attempt() {
        let (status, client, err) = create(&options(&[
            ("Endpoint", "http://localhost:1"),
            ("Timeout", "not a duration"),
        ]));

        assert_eq!(status, crate::status::INVALID_CONFIG);
        assert!(client.is_null());
        // SAFETY: produced by the failed call above, freed exactly once.
        unsafe { crate::error::ak_bytes_free(err) };
    }

    #[test]
    fn a_misspelled_option_is_reported_by_name() {
        let (status, client, err) = create(&options(&[
            ("Endpoint", "http://localhost:1"),
            ("Timeuot", "30s"),
        ]));

        assert_eq!(status, crate::status::INVALID_CONFIG);
        assert!(client.is_null());
        assert!(!err.owner.is_null());
        // SAFETY: produced by the failed call above.
        let message = unsafe { std::slice::from_raw_parts(err.ptr, err.len) };
        let message = String::from_utf8_lossy(message);
        assert!(
            message.contains("Timeuot"),
            "the error should name the option: {message}"
        );
        // SAFETY: freed exactly once.
        unsafe { crate::error::ak_bytes_free(err) };
    }

    #[test]
    fn a_client_certificate_that_does_not_match_its_key_is_rejected_by_name() {
        // Both halves are real and parse cleanly; only `rustls` can see that they are not a pair, and
        // it does so while building the TLS configuration — before any socket is opened. Worth pinning
        // down, because a customer who mixes up two deployments' files hits exactly this, and the
        // error has to say so rather than look like an unreachable endpoint.
        let (cert_pem, _) = crate::test_support::certificate();
        let (_, unrelated_key_pem) = crate::test_support::certificate();

        let blob = options(&[("Endpoint", "https://localhost:1")]);
        let mut out: *mut ak_client = std::ptr::null_mut();
        let mut err = ak_bytes::EMPTY;
        // SAFETY: the two PEM buffers and the blob are live across the call, and both out-parameters
        // point at live locals.
        let status = unsafe {
            ak_client_create(
                blob.as_ptr(),
                blob.len(),
                ak_bytes_in {
                    ptr: cert_pem.as_ptr(),
                    len: cert_pem.len(),
                },
                ak_bytes_in {
                    ptr: unrelated_key_pem.as_ptr(),
                    len: unrelated_key_pem.len(),
                },
                empty_in(),
                std::ptr::addr_of_mut!(out),
                std::ptr::addr_of_mut!(err),
            )
        };

        assert_eq!(status, crate::status::CONNECTION_FAILED);
        assert!(out.is_null());
        // SAFETY: produced by the failed call above.
        let message = unsafe { std::slice::from_raw_parts(err.ptr, err.len) };
        let message = String::from_utf8_lossy(message).to_lowercase();
        assert!(
            message.contains("key") || message.contains("certificate"),
            "the mismatch should be named, not reported as a connection problem: {message}"
        );
        // SAFETY: freed exactly once.
        unsafe { crate::error::ak_bytes_free(err) };
    }

    #[test]
    fn a_null_out_parameter_is_rejected_rather_than_dereferenced() {
        let blob = options_for("http://localhost:1");
        let mut err = ak_bytes::EMPTY;
        // SAFETY: deliberately passing a null `out`, which the function documents as rejected.
        let status = unsafe {
            ak_client_create(
                blob.as_ptr(),
                blob.len(),
                empty_in(),
                empty_in(),
                empty_in(),
                std::ptr::null_mut(),
                std::ptr::addr_of_mut!(err),
            )
        };
        assert_eq!(status, crate::status::NULL_ARGUMENT);
        // SAFETY: freed exactly once.
        unsafe { crate::error::ak_bytes_free(err) };
    }

    #[test]
    fn freeing_null_is_a_no_op() {
        // SAFETY: null is explicitly allowed.
        unsafe { ak_client_free(std::ptr::null_mut()) };
    }

    #[test]
    fn a_double_free_is_rejected_instead_of_touching_freed_memory() {
        let address = spawn_server();
        let (status, client, err) = create(&options_for(&format!("http://{address}")));
        assert_eq!(status, crate::status::OK);
        // SAFETY: the zeroed value on success.
        unsafe { crate::error::ak_bytes_free(err) };

        // SAFETY: a live handle.
        unsafe { ak_client_free(client) };
        // If this dereferenced the freed allocation, the test would corrupt memory or crash rather
        // than simply completing.
        unsafe { ak_client_free(client) };
    }
}
