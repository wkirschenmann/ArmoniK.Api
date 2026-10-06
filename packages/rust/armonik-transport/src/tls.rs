//! The rustls client configuration both connectors secure a connection with.
//!
//! [`crate::https_connector`] and the engine's [`crate::http2`] read their settings from different
//! vocabularies, so what they share is the step after: which roots, whether anything is verified
//! at all, and which identity is presented.

use std::sync::Arc;

use hyper_rustls::ConfigBuilderExt;
use rustls::pki_types::{CertificateDer, IpAddr, PrivateKeyDer, ServerName};

/// What a server certificate is verified against.
pub(crate) enum Trust {
    /// The operating system's roots.
    System,
    /// These roots and no others.
    Roots(Vec<CertificateDer<'static>>),
    /// Nothing: any certificate is accepted.
    Anything,
}

/// The step that refused, with what refused it.
#[derive(Debug)]
pub(crate) enum Refused {
    /// The crypto provider admits none of the protocol versions asked for.
    Protocols(rustls::Error),
    /// A root that is not a certificate rustls can anchor a chain to.
    Root(rustls::Error),
    /// The system's store could not be read.
    SystemRoots(std::io::Error),
    /// A client certificate and key rustls will not present together.
    Identity(rustls::Error),
}

/// A client configuration that trusts `trust` and presents `identity`.
pub(crate) fn client_config(
    trust: Trust,
    identity: Option<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)>,
) -> Result<rustls::ClientConfig, Refused> {
    // The process's provider when it installed one, so a host that chose its crypto keeps it.
    let provider = rustls::crypto::CryptoProvider::get_default()
        .cloned()
        .unwrap_or_else(|| Arc::new(rustls::crypto::ring::default_provider()));

    let builder = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(Refused::Protocols)?;

    let builder = match trust {
        Trust::Anything => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(crate::utils::InsecureCertVerifier)),
        Trust::Roots(roots) => {
            let mut store = rustls::RootCertStore::empty();
            for root in roots {
                store.add(root).map_err(Refused::Root)?;
            }
            builder.with_root_certificates(store)
        }
        Trust::System => builder.with_native_roots().map_err(Refused::SystemRoots)?,
    };

    match identity {
        Some((chain, key)) => builder
            .with_client_auth_cert(chain, key)
            .map_err(Refused::Identity),
        None => Ok(builder.with_no_client_auth()),
    }
}

/// The name a server certificate is verified against, from a host as an authority writes it.
///
/// `http` reports the host of an IPv6 authority with its brackets, as `[::1]`, while a
/// [`ServerName`] is the address alone. Brackets delimit an IP literal and nothing else, so what
/// stands between them has to parse as an address rather than fall back to being read as a name.
pub(crate) fn server_name(host: &str) -> Option<ServerName<'static>> {
    match host.strip_prefix('[').and_then(|ip| ip.strip_suffix(']')) {
        Some(literal) => IpAddr::try_from(literal).ok().map(ServerName::from),
        None => ServerName::try_from(host).ok().map(|name| name.to_owned()),
    }
}
