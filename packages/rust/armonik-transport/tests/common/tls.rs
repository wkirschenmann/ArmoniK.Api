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
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::WebPkiClientVerifier;

use super::echo::{answer, loopback};

/// A certificate authority of the run's own, and the leaves it signs.
pub struct Pki {
    issuer: CertifiedIssuer<'static, KeyPair>,
    /// The authorities between this one and the root, this one first, as PEM; empty for a root.
    above: Vec<String>,
}

/// A certificate, the authorities between it and the root, and its key.
pub struct Leaf {
    pub chain: Vec<CertificateDer<'static>>,
    pub key: PrivateKeyDer<'static>,
    /// The chain, as the PEM file `CertPem` names.
    pub chain_pem: String,
    /// The key, as the PEM file `KeyPem` names.
    pub key_pem: String,
}

impl Leaf {
    /// The chain and the key as a PKCS#12 bundle protected by `password`.
    pub fn pkcs12(&self, password: &str) -> Vec<u8> {
        let PrivateKeyDer::Pkcs8(key) = &self.key else {
            panic!("the test CA issues PKCS#8 keys");
        };
        let chain = p12_keystore::PrivateKeyChain::new(
            [1u8].as_slice(),
            p12_keystore::PrivateKey::from_der(key.secret_pkcs8_der()).expect("a PKCS#8 key"),
            self.chain.iter().map(|certificate| {
                p12_keystore::Certificate::from_der(certificate.as_ref()).expect("a certificate")
            }),
        );
        let mut store = p12_keystore::KeyStore::new();
        store.add_entry(
            "identity",
            p12_keystore::KeyStoreEntry::PrivateKeyChain(chain),
        );
        store.writer(password).write().expect("a bundle")
    }
}

/// A directory of the test's own, removed with the keys it holds however the test ends.
pub struct Scratch(std::path::PathBuf);

impl Scratch {
    /// Empty: what a killed run under a recycled pid left there goes first.
    pub fn new(name: &str) -> Self {
        let directory =
            std::env::temp_dir().join(format!("armonik-transport-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("a scratch directory");
        Self(directory)
    }

    /// Writes `content` as `name`, and answers its path as an option names it.
    pub fn file(&self, name: &str, content: &[u8]) -> String {
        let path = self.0.join(name);
        std::fs::write(&path, content).expect("a scratch file");
        path.to_string_lossy().into_owned()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
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
        Self {
            issuer,
            above: Vec::new(),
        }
    }

    /// An authority this one signs, whose leaves a peer trusting only the root can verify only
    /// when it is given the intermediate as well.
    pub fn intermediate(&self) -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).expect("CA parameters");
        params
            .distinguished_name
            .push(DnType::CommonName, "armonik-transport test intermediate");
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let issuer = CertifiedIssuer::signed_by(
            params,
            KeyPair::generate().expect("an intermediate key"),
            &self.issuer,
        )
        .expect("an intermediate the root signed");
        let mut above = vec![issuer.pem()];
        above.extend(self.above.iter().cloned());
        Self { issuer, above }
    }

    /// This authority's own certificate: the root for one `new` made, the intermediate for one
    /// `intermediate` made.
    pub fn root(&self) -> CertificateDer<'static> {
        self.issuer.der().clone()
    }

    /// The same certificate, as the PEM a `CaCertPath` file holds.
    pub fn root_pem(&self) -> String {
        self.issuer.pem()
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
        let chain_pem = std::iter::once(certificate.pem())
            .chain(self.above.iter().cloned())
            .collect::<String>();
        Leaf {
            chain: CertificateDer::pem_slice_iter(chain_pem.as_bytes())
                .collect::<Result<_, _>>()
                .expect("the chain just written"),
            key: PrivateKeyDer::Pkcs8(key.serialize_der().into()),
            chain_pem,
            key_pem: key.serialize_pem(),
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
