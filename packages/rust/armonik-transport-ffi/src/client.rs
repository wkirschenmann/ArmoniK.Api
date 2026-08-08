//! The connection pool a request is issued on.
//!
//! An `ak_client` is a `hyper_util` legacy client over the connector `armonik-transport` builds:
//! TCP settings, TLS, mTLS and proxying all come from there, and nothing about them is re-decided
//! here. Creation is synchronous and lazy - the configuration is parsed and the connector is
//! assembled, but no socket is opened until the first request, which is what a host application's
//! UI thread needs.

use std::sync::{Arc, OnceLock};

use armonik_transport::reexports::http;
use armonik_transport::reexports::hyper::body::Incoming;
use armonik_transport::reexports::hyper_util::client::legacy::Client;
use armonik_transport::reexports::hyper_util::rt::{TokioExecutor, TokioTimer};
use armonik_transport::{Connector, HttpConfig};

use crate::error::{ak_bytes, FfiError};
use crate::handle::Registry;
use crate::request::RequestBody;

/// The pool a request is issued on.
///
/// Writable at all because `armonik-transport` names the connector stack it builds: without that
/// alias the concrete `Client<C, _>` could not be spelled, and Rust has no way to infer the type of
/// a struct field.
pub(crate) type Pool = Client<Connector, RequestBody>;

/// A connection pool, and the options every request on it inherits.
///
/// Handed to the caller as an opaque pointer. Cloning the inner `Arc` into each request is what
/// lets a request outlive [`ak_client_free`]: the pool goes away when the last user does.
pub struct ak_client {
    pub(crate) pool: Arc<Pool>,
    /// The whole-request timeout from the configuration, applied by the driving task. Nothing at
    /// the `hyper_util` level implements it.
    pub(crate) timeout: Option<std::time::Duration>,
    /// Sent as `user-agent` on every request that does not carry one of its own.
    pub(crate) user_agent: Option<http::HeaderValue>,
}

fn live() -> &'static Registry<ak_client> {
    static LIVE: OnceLock<Registry<ak_client>> = OnceLock::new();
    LIVE.get_or_init(Registry::new)
}

/// A counted reference to a live client, or `None`.
///
/// Counted rather than borrowed: the caller may free the client from another thread at any moment,
/// and a request being started has to keep reading it either way.
pub(crate) fn get(client: *const ak_client) -> Option<Arc<ak_client>> {
    live().get(client)
}

/// Build a client from a JSON configuration document.
///
/// `config_json` names the flat options of `armonik_transport::HttpConfig` (`Endpoint`,
/// `AllowUnsafeConnection`, `CaCert`, `Proxy*`, `Tcp*`, `Http2*`, ...). Everything it accepts is
/// documented by the JSON schema that crate generates.
///
/// Synchronous and lazy: this validates the options and assembles the connector, and opens no
/// connection. A failure here is a configuration failure, reported immediately with its whole cause
/// chain flattened into `out_err`. It touches no runtime, so it is callable from anywhere, an event
/// callback included.
///
/// # Safety
///
/// `config_json` must point to `len` readable bytes. `out` must be a writable `ak_client*`, and
/// receives a handle to be released with exactly one [`ak_client_free`]. `out_err`, when non-null,
/// must be a writable [`ak_bytes`] and receives a message to release with [`crate::ak_bytes_free`].
#[no_mangle]
pub unsafe extern "C" fn ak_client_create(
    config_json: *const u8,
    len: usize,
    out: *mut *mut ak_client,
    out_err: *mut ak_bytes,
) -> i32 {
    crate::guard::catch_unwind_status(out_err, || {
        if out.is_null() {
            return FfiError::NullArgument("out").into_ffi_result(out_err);
        }
        // SAFETY: documented as writable by this function's contract.
        unsafe { *out = std::ptr::null_mut() };
        if config_json.is_null() {
            return FfiError::NullArgument("config_json").into_ffi_result(out_err);
        }

        // SAFETY: forwarded from this function's contract.
        let bytes = unsafe { std::slice::from_raw_parts(config_json, len) };
        let client = match build(bytes) {
            Ok(client) => client,
            Err(error) => return error.into_ffi_result(out_err),
        };

        // SAFETY: checked non-null above.
        unsafe { *out = live().insert(client).cast_mut() };
        crate::status::OK
    })
}

fn build(config_json: &[u8]) -> Result<ak_client, FfiError> {
    let text = std::str::from_utf8(config_json).map_err(|_| FfiError::InvalidUtf8)?;
    let config: HttpConfig =
        serde_json::from_str(text).map_err(|error| FfiError::InvalidJson(error.to_string()))?;

    let timeout = config.timeout;
    let user_agent = config.user_agent.clone();
    let http2 = config.http2;

    let connector = armonik_transport::https_connector(config)?;

    let mut builder = Client::builder(TokioExecutor::new());
    // Without these three the client is unusable for gRPC. `http2_only` because an h2c endpoint has
    // no ALPN to negotiate with and would otherwise be spoken HTTP/1.1 to; the timers because the
    // HTTP/2 keep-alive and the pool's idle sweep both panic when they need a timer and none was
    // installed.
    builder
        .http2_only(true)
        .timer(TokioTimer::new())
        .pool_timer(TokioTimer::new());

    if let Some(interval) = http2.keep_alive_interval {
        builder.http2_keep_alive_interval(interval);
    }
    if let Some(timeout) = http2.keep_alive_timeout {
        builder.http2_keep_alive_timeout(timeout);
    }
    builder.http2_keep_alive_while_idle(http2.keep_alive_while_idle);
    if let Some(max) = http2.max_header_list_size {
        builder.http2_max_header_list_size(max);
    }

    Ok(ak_client {
        pool: Arc::new(builder.build::<_, RequestBody>(connector)),
        timeout,
        user_agent,
    })
}

/// Release a client.
///
/// Requests already in flight keep the pool alive and run to their `COMPLETED` event, and a call
/// already inside another entry point finishes normally: this gives up the caller's reference, not
/// necessarily the last one.
///
/// # Safety
///
/// `client` must be a handle from [`ak_client_create`] that has not been freed, or null.
#[no_mangle]
pub unsafe extern "C" fn ak_client_free(client: *mut ak_client) {
    crate::guard::catch_unwind_void(|| {
        if client.is_null() {
            return;
        }
        drop(live().remove(client));
    });
}

/// The `Incoming` body every response arrives with, named once for the reactor.
pub(crate) type ResponseBody = Incoming;

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a client and free it, reporting the status and the message.
    fn create(config: &str) -> (i32, String) {
        let mut client: *mut ak_client = std::ptr::null_mut();
        let mut err = ak_bytes::EMPTY;
        // SAFETY: both out-parameters are live locals, and the buffer outlives the call.
        let status = unsafe {
            ak_client_create(
                config.as_ptr(),
                config.len(),
                std::ptr::addr_of_mut!(client),
                std::ptr::addr_of_mut!(err),
            )
        };
        let message = if err.ptr.is_null() {
            String::new()
        } else {
            // SAFETY: written by the call above, freed immediately after.
            let seen = unsafe { std::slice::from_raw_parts(err.ptr, err.len) }.to_vec();
            unsafe { crate::ak_bytes_free(err) };
            String::from_utf8_lossy(&seen).into_owned()
        };
        if !client.is_null() {
            // SAFETY: produced by the call above.
            unsafe { ak_client_free(client) };
        }
        (status, message)
    }

    #[test]
    fn a_valid_configuration_produces_a_client_without_connecting() {
        // Nothing listens on this port. Creation still succeeds: the pool is lazy, which is the
        // whole reason a host application may call this from a UI thread.
        let (status, message) = create(r#"{"Endpoint": "http://127.0.0.1:1/"}"#);
        assert_eq!(status, crate::status::OK, "{message}");
    }

    #[test]
    fn a_document_that_is_not_json_is_refused_with_a_message() {
        let (status, message) = create("not json at all");
        assert_eq!(status, crate::status::INVALID_CONFIG);
        assert!(!message.is_empty(), "the caller has nothing else to go on");
    }

    #[test]
    fn an_unset_endpoint_is_refused_by_the_transport_and_named_in_the_message() {
        let (status, message) = create("{}");
        assert_eq!(status, crate::status::CONNECTION_FAILED);
        assert!(
            message.contains("`Endpoint` is not set"),
            "the option at fault has to survive the flattening: {message}"
        );
    }

    #[test]
    fn an_unreadable_certificate_is_reported_with_its_cause_rather_than_a_summary() {
        let (status, message) =
            create(r#"{"Endpoint": "https://localhost:443/", "CaCert": "no/such/file.pem"}"#);
        assert_eq!(status, crate::status::INVALID_CONFIG);
        assert!(
            message.contains("no/such/file.pem"),
            "a message that stopped at the outer error would not say which file: {message}"
        );
    }

    #[test]
    fn a_null_configuration_is_rejected_rather_than_dereferenced() {
        let mut client: *mut ak_client = std::ptr::null_mut();
        // SAFETY: `out` is a live local; the null `config_json` is the case under test.
        let status = unsafe {
            ak_client_create(
                std::ptr::null(),
                0,
                std::ptr::addr_of_mut!(client),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(status, crate::status::NULL_ARGUMENT);
        assert!(client.is_null());
    }

    #[test]
    fn freeing_a_client_twice_is_refused_rather_than_a_double_free() {
        let mut client: *mut ak_client = std::ptr::null_mut();
        let config = r#"{"Endpoint": "http://127.0.0.1:1/"}"#;
        // SAFETY: live out-parameter, live buffer.
        let status = unsafe {
            ak_client_create(
                config.as_ptr(),
                config.len(),
                std::ptr::addr_of_mut!(client),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(status, crate::status::OK);

        // SAFETY: the first frees, the second must be caught by the live set.
        unsafe {
            ak_client_free(client);
            ak_client_free(client);
        }
    }
}
