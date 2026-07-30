//! Building an [`armonik_transport::ClientConfig`] from what the caller passed to
//! [`crate::ak_client_create`].
//!
//! # Why options travel as a blob
//!
//! The options arrive as a key/value blob ([`crate::blob`]) rather than a `#[repr(C)]` struct with a
//! field per option. A struct would be marginally simpler for a caller to fill in, but every new
//! option would change its memory layout, and a layout mismatch between this library and the caller's
//! declaration of it reads one field as another rather than failing — the worst possible way for a
//! configuration bug to present itself. With a blob, adding an option is additive, and an option the
//! native side does not recognise is *reported* rather than guessed at.
//!
//! The names are exactly [`armonik_transport::ClientConfigArgs::OPTION_NAMES`] — the suffixes of the
//! `GrpcClient__*` environment variables, and so the same names ArmoniK's client configuration uses
//! everywhere else. One vocabulary across every surface, defined once in `armonik-transport` and never
//! restated here, so this module cannot fall out of step with it.
//!
//! # Why certificates do not
//!
//! `cert_pem`/`key_pem`/`ca_cert` are passed separately, as PEM bytes rather than the file paths
//! `ClientConfigArgs` expects. That is deliberate: it lets a caller hand over material taken from a
//! PKCS#12 file or the operating system's certificate store without ever writing a private key to
//! disk. Keeping them out of the options blob also keeps the private key structurally apart from the
//! values this crate is free to log.

use armonik_transport::reexports::rustls::pki_types::pem::PemObject;
use armonik_transport::reexports::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use armonik_transport::{ClientConfig, ClientConfigArgs};

use crate::error::{ak_bytes_in, FfiError};

/// The option names that name certificate *files* in `ClientConfigArgs` but arrive as bytes here.
///
/// Cleared before `from_config_args` runs so it never opens a path: on this ABI the material comes
/// through [`Certificates`] instead.
const CERTIFICATE_OPTIONS: &[&str] = &["CertPem", "KeyPem", "CaCert"];

/// The certificate material for a client, as PEM bytes.
///
/// Each field is empty (null pointer or zero length) when absent. `cert_pem` and `key_pem` must
/// either both be present or both be absent.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Certificates {
    pub(crate) cert_pem: ak_bytes_in,
    pub(crate) key_pem: ak_bytes_in,
    pub(crate) ca_cert_pem: ak_bytes_in,
}

/// Build a client configuration from an options blob and the certificate material.
///
/// # Safety
///
/// `options` must point to `options_len` valid bytes holding a [`crate::blob`], and every field of
/// `certificates` must satisfy [`ak_bytes_in`]'s contract, for the duration of this call.
pub(crate) unsafe fn build(
    options: *const u8,
    options_len: usize,
    certificates: Certificates,
) -> Result<ClientConfig, FfiError> {
    // SAFETY: forwarded from this function's own contract.
    let pairs = unsafe { crate::blob::decode(options, options_len) }?;

    let mut args = ClientConfigArgs::default();
    for (key, value) in pairs {
        let key = std::str::from_utf8(key).map_err(|_| FfiError::InvalidUtf8)?;
        let value = std::str::from_utf8(value).map_err(|_| FfiError::InvalidUtf8)?;
        args.set(key, value)?;
    }

    // A caller that put a certificate path in the blob meant something this ABI does not accept;
    // clearing these keeps `from_config_args` from reading a file the caller never intended.
    for name in CERTIFICATE_OPTIONS {
        args.set(name, "")?;
    }

    let mut config = ClientConfig::from_config_args(args)?;

    // SAFETY: forwarded from this function's own contract.
    let (cert_pem, key_pem, ca_cert_pem) = unsafe {
        (
            certificates.cert_pem.as_slice(),
            certificates.key_pem.as_slice(),
            certificates.ca_cert_pem.as_slice(),
        )
    };

    config.identity = match (cert_pem.is_empty(), key_pem.is_empty()) {
        (true, true) => None,
        (false, false) => {
            let cert = CertificateDer::from_pem_slice(cert_pem)
                .map_err(|source| FfiError::InvalidCertPem(source.to_string()))?;
            let key = PrivateKeyDer::from_pem_slice(key_pem)
                .map_err(|source| FfiError::InvalidKeyPem(source.to_string()))?;
            Some((cert, key))
        }
        _ => return Err(FfiError::MismatchedIdentity),
    };

    config.cacert = if ca_cert_pem.is_empty() {
        None
    } else {
        Some(
            CertificateDer::from_pem_slice(ca_cert_pem)
                .map_err(|source| FfiError::InvalidCaCertPem(source.to_string()))?,
        )
    };

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::certificate;

    fn view(s: &[u8]) -> ak_bytes_in {
        ak_bytes_in {
            ptr: s.as_ptr(),
            len: s.len(),
        }
    }

    fn empty() -> ak_bytes_in {
        ak_bytes_in {
            ptr: std::ptr::null(),
            len: 0,
        }
    }

    fn no_certificates() -> Certificates {
        Certificates {
            cert_pem: empty(),
            key_pem: empty(),
            ca_cert_pem: empty(),
        }
    }

    /// Encode an options blob the way a caller will.
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

    fn build_from(
        pairs: &[(&str, &str)],
        certificates: Certificates,
    ) -> Result<ClientConfig, FfiError> {
        let blob = options(pairs);
        // SAFETY: `blob` and every certificate view are live across the call.
        unsafe { build(blob.as_ptr(), blob.len(), certificates) }
    }

    #[test]
    fn a_minimal_config_builds() {
        let config = build_from(&[("Endpoint", "https://localhost:5001")], no_certificates())
            .expect("build");
        assert_eq!(config.endpoint.to_string(), "https://localhost:5001/");
    }

    #[test]
    fn options_reach_the_transport_crates_own_parsing() {
        let config = build_from(
            &[
                ("Endpoint", "https://localhost:5001"),
                ("Timeout", "30s"),
                ("MaxAttempts", "3"),
                ("AllowUnsafeConnection", "true"),
                ("ReusePorts", "1"),
            ],
            no_certificates(),
        )
        .expect("build");

        assert_eq!(config.timeout, Some(std::time::Duration::from_secs(30)));
        assert_eq!(config.retry.expect("a retry policy").max_attempts, 3);
        assert!(config.allow_unsafe_connection);
        assert!(config.reuse_ports);
    }

    #[test]
    fn absent_options_take_their_defaults() {
        // Only the endpoint is given; everything else must fall back rather than being required.
        let config = build_from(&[("Endpoint", "https://localhost:5001")], no_certificates())
            .expect("build");

        assert_eq!(config.retry, None);
        assert!(!config.reuse_ports);
        assert!(!config.allow_unsafe_connection);
        assert_eq!(config.timeout, None);
        assert_eq!(
            config.proxy.source,
            armonik_transport::ProxySource::Disabled
        );
    }

    #[test]
    fn an_endpoint_is_required() {
        // There is nothing sensible to default an endpoint to, so an options blob without one has to
        // fail here rather than produce a client pointed at nowhere.
        // SAFETY: the null/zero case is a valid (if empty) blob.
        let error = unsafe { build(std::ptr::null(), 0, no_certificates()) }
            .expect_err("a config with no endpoint must be rejected");
        assert!(matches!(error, FfiError::Config(_)));
    }

    #[test]
    fn an_unknown_option_is_reported_rather_than_ignored() {
        // The whole reason for choosing a blob over a `repr(C)` struct: a name the native side does
        // not know must be named in an error, not silently read as some neighbouring field.
        let error = build_from(
            &[("Endpoint", "https://localhost:5001"), ("EndPoint", "typo")],
            no_certificates(),
        )
        .expect_err("a misspelled option must be rejected");

        assert!(
            error.to_string().contains("not a client option"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn an_invalid_option_value_is_reported() {
        let error = build_from(
            &[
                ("Endpoint", "https://localhost:5001"),
                ("Timeout", "not a duration"),
            ],
            no_certificates(),
        )
        .expect_err("an unparseable duration must be rejected");
        assert!(matches!(error, FfiError::Config(_)));
    }

    #[test]
    fn a_non_utf8_key_or_value_is_reported() {
        // The blob format carries opaque bytes, so this rejection has to come from here.
        let mut blob = 1u32.to_ne_bytes().to_vec();
        blob.extend_from_slice(&2u32.to_ne_bytes());
        blob.extend_from_slice(&[0xff, 0xfe]);
        blob.extend_from_slice(&0u32.to_ne_bytes());

        // SAFETY: `blob` is live across the call.
        let error = unsafe { build(blob.as_ptr(), blob.len(), no_certificates()) }
            .expect_err("invalid UTF-8 must be rejected");
        assert!(matches!(error, FfiError::InvalidUtf8));
    }

    #[test]
    fn a_cert_and_key_pair_is_parsed_and_attached() {
        let (cert_pem, key_pem) = certificate();
        let config = build_from(
            &[("Endpoint", "https://localhost:5001")],
            Certificates {
                cert_pem: view(cert_pem.as_bytes()),
                key_pem: view(key_pem.as_bytes()),
                ca_cert_pem: empty(),
            },
        )
        .expect("build");
        assert!(config.identity.is_some());
    }

    #[test]
    fn a_cert_without_a_key_or_a_key_without_a_cert_is_rejected() {
        let (cert_pem, key_pem) = certificate();

        for certificates in [
            Certificates {
                cert_pem: view(cert_pem.as_bytes()),
                key_pem: empty(),
                ca_cert_pem: empty(),
            },
            Certificates {
                cert_pem: empty(),
                key_pem: view(key_pem.as_bytes()),
                ca_cert_pem: empty(),
            },
        ] {
            let error = build_from(&[("Endpoint", "https://localhost:5001")], certificates)
                .expect_err("half an identity must be rejected");
            assert!(matches!(error, FfiError::MismatchedIdentity));
        }
    }

    #[test]
    fn malformed_pem_is_reported_rather_than_panicking() {
        let error = build_from(
            &[("Endpoint", "https://localhost:5001")],
            Certificates {
                cert_pem: view(b"not a certificate"),
                key_pem: view(b"not a key"),
                ca_cert_pem: empty(),
            },
        )
        .expect_err("malformed PEM must be rejected");
        assert!(matches!(error, FfiError::InvalidCertPem(_)));
    }

    #[test]
    fn a_ca_cert_is_parsed_and_attached() {
        let (ca_pem, _key_pem) = certificate();
        let config = build_from(
            &[("Endpoint", "https://localhost:5001")],
            Certificates {
                cert_pem: empty(),
                key_pem: empty(),
                ca_cert_pem: view(ca_pem.as_bytes()),
            },
        )
        .expect("build");
        assert!(config.cacert.is_some());
    }

    #[test]
    fn certificate_options_in_the_blob_do_not_become_file_paths() {
        // `ClientConfigArgs` reads `CaCert` as a path off disk. On this ABI the material comes as
        // bytes instead, so a caller that put a path in the blob must not have it silently read —
        // which, for a path that does not exist, would surface as a confusing I/O error.
        let config = build_from(
            &[
                ("Endpoint", "https://localhost:5001"),
                ("CaCert", "/nonexistent/ca.pem"),
            ],
            no_certificates(),
        )
        .expect("the path must be ignored, not opened");
        assert!(config.cacert.is_none());
    }
}
