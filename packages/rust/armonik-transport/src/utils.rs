//! An error's causes as one line, an endpoint as a message may print it, and the one
//! deliberately-insecure certificate verifier that accepting any server selects.

/// An error and its causes, rendered into one line.
pub(crate) fn chain(error: &(dyn std::error::Error + 'static), separator: &str) -> String {
    snafu::ChainCompat::new(error)
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(separator)
}

/// An endpoint as an error or a span may print it: scheme, host and port, and nothing else.
///
/// A URI can carry `user:password@`, and every message that took `{endpoint}` put it in the
/// caller's log. `http2::dialable` refuses such an endpoint outright, but a config built by hand is
/// not checked, and an error is not the place to find that out. Public because an endpoint a caller
/// holds never met the check that refuses userinfo, and its holder needs this to say where it is
/// connecting.
pub fn safe_endpoint(endpoint: &http::Uri) -> String {
    let scheme = endpoint.scheme_str().unwrap_or("http");
    match (endpoint.host(), endpoint.port_u16()) {
        (Some(host), Some(port)) => format!("{scheme}://{host}:{port}"),
        (Some(host), None) => format!("{scheme}://{host}"),
        (None, _) => format!("{scheme}://<no host>"),
    }
}

#[derive(Debug)]
pub(crate) struct InsecureCertVerifier;

impl rustls::client::danger::ServerCertVerifier for InsecureCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA1,
            rustls::SignatureScheme::ECDSA_SHA1_Legacy,
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::ED448,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What nine call sites rely on, none of which measured it.
    #[test]
    fn a_rendered_endpoint_carries_no_userinfo() {
        let rendered =
            |uri: &str| safe_endpoint(&http::Uri::try_from(uri).expect("a uri to render"));

        assert_eq!(
            rendered("https://alice:s3cret@example.test:5001/path?q=1"),
            "https://example.test:5001"
        );
        assert_eq!(
            rendered("https://alice:s3cret@example.test"),
            "https://example.test"
        );
        assert_eq!(
            rendered("http://example.test:5001"),
            "http://example.test:5001"
        );
    }

    /// A URI with no host is rendered rather than passed through: the string is what would be
    /// printed, and there is nothing in it this can promise is not a secret.
    #[test]
    fn an_endpoint_with_no_host_names_the_absence() {
        assert_eq!(
            safe_endpoint(&http::Uri::try_from("/only/a/path").expect("a uri")),
            "http://<no host>"
        );
    }
}
