//! A server that answers over TLS, under certificates made for the run.
//!
//! Made rather than committed, so no key sits in the repository and none expires under a test.

use std::convert::Infallible;
use std::sync::Arc;

use armonik_transport::reexports::hyper;
use armonik_transport::reexports::hyper_util::rt::{TokioExecutor as HyperTokio, TokioIo};
use armonik_transport::reexports::rustls;
use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::WebPkiClientVerifier;

use super::echo::{answer, loopback};

/// A certificate authority of the run's own, and the leaves it signs.
pub struct Pki {
    issuer: CertifiedIssuer<'static, KeyPair>,
}

/// A certificate and its key.
pub struct Leaf {
    pub chain: Vec<CertificateDer<'static>>,
    pub key: PrivateKeyDer<'static>,
}

impl Pki {
    pub fn new() -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).expect("CA parameters");
        params
            .distinguished_name
            .push(DnType::CommonName, "armonik-transport test CA");
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let issuer = CertifiedIssuer::self_signed(params, KeyPair::generate().expect("a CA key"))
            .expect("a self-signed CA");
        Self { issuer }
    }

    /// The root a client verifies this authority's leaves against.
    pub fn root(&self) -> CertificateDer<'static> {
        self.issuer.der().clone()
    }

    /// A server certificate for `names`, which may be DNS names or IP addresses.
    pub fn server(&self, names: &[&str]) -> Leaf {
        self.leaf(names, ExtendedKeyUsagePurpose::ServerAuth)
    }

    /// A client certificate.
    pub fn client(&self) -> Leaf {
        self.leaf(&["client.test"], ExtendedKeyUsagePurpose::ClientAuth)
    }

    fn leaf(&self, names: &[&str], usage: ExtendedKeyUsagePurpose) -> Leaf {
        let key = KeyPair::generate().expect("a leaf key");
        let mut params = CertificateParams::new(
            names
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>(),
        )
        .expect("leaf parameters");
        params.extended_key_usages = vec![usage];
        let certificate = params
            .signed_by(&key, &self.issuer)
            .expect("a leaf the CA signed");
        Leaf {
            chain: vec![certificate.der().clone()],
            key: PrivateKeyDer::Pkcs8(key.serialize_der().into()),
        }
    }
}

/// The echo service, over TLS, on an ephemeral loopback port.
pub struct TlsServer {
    /// `https://127.0.0.1:<port>`.
    pub endpoint: String,
}

impl TlsServer {
    /// Serves as `leaf`, asking for a client certificate this authority signed when `clients`
    /// names one.
    pub async fn start(leaf: Leaf, clients: Option<&Pki>) -> Self {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let builder = rustls::ServerConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .expect("protocol versions");
        let builder = match clients {
            Some(pki) => {
                let mut roots = rustls::RootCertStore::empty();
                roots.add(pki.root()).expect("the client CA");
                let verifier =
                    WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider)
                        .build()
                        .expect("a client verifier");
                builder.with_client_cert_verifier(verifier)
            }
            None => builder.with_no_client_auth(),
        };
        let mut config = builder
            .with_single_cert(leaf.chain, leaf.key)
            .expect("the server's certificate and key");
        config.alpn_protocols = vec![b"h2".to_vec()];
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));

        let (listener, plain) = loopback().await;
        let endpoint = plain.replacen("http://", "https://", 1);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    // A handshake this server refuses is the test's subject, not a failure here.
                    let Ok(stream) = acceptor.accept(stream).await else {
                        return;
                    };
                    let service = hyper::service::service_fn(|request| async {
                        Ok::<_, Infallible>(answer(request).await)
                    });
                    let _ = hyper::server::conn::http2::Builder::new(HyperTokio::new())
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });

        Self { endpoint }
    }
}
