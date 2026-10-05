//! A client identity and a root resolved from a Windows certificate store.
//!
//! The certificates go into stores of the current user that only these tests use, one per test so
//! they run at once, and are deleted from them when the test ends. The keys a PKCS#12 import
//! persists stay in the user's key store: nothing this crate links removes one.
#![cfg(windows)]

mod common;

use std::time::Duration;

use armonik_transport::grpc::{CallStartOptions, GrpcChannelConfig, GrpcStatus, GrpcStatusCode};
use armonik_transport::http2::{TlsConfig, TransportConfig};
use armonik_transport::options::{
    ClientCertificate, ServerVerification, StoreCertificate, StoreSearch, TlsOptions,
};
use bytes::Bytes;
use common::echo::{channel_with, unary, ECHO};
use common::tls::{Leaf, Pki, TlsServer};
use http::Uri;
use schannel::cert_context::{CertContext, HashAlgorithm};
use schannel::cert_store::{CertAdd, CertStore, PfxImportOptions};

/// A store of the current user that one test owns, emptied of what the test added when it ends.
struct TestStore {
    name: &'static str,
    added: Vec<Vec<u8>>,
}

/// Deletes the certificates of `store` whose fingerprint is one of `fingerprints`.
///
/// Through the store opened again: the context `add_cert` answers outlives the store handle it
/// came from, and a delete through it fails.
fn delete_from(store: CertStore, fingerprints: &[Vec<u8>]) {
    for certificate in store.certs() {
        let fingerprint = certificate
            .fingerprint(HashAlgorithm::sha1())
            .unwrap_or_default();
        if fingerprints.contains(&fingerprint) {
            let _ = certificate.delete();
        }
    }
}

impl TestStore {
    fn new(name: &'static str) -> Self {
        let store = Self {
            name,
            added: Vec::new(),
        };
        // What a run that was killed left behind.
        let open = CertStore::open_current_user(name).expect("a test store");
        for certificate in open.certs() {
            let _ = certificate.delete();
        }
        store
    }

    /// Adds `leaf` with its key, which the store may export when `exportable`, under `friendly`.
    fn add(&mut self, leaf: &Leaf, friendly: &str, exportable: bool) -> CertContext {
        let imported = PfxImportOptions::new()
            .password("import")
            .exportable_private_key(exportable)
            .import(&leaf.pkcs12("import"))
            .expect("a bundle Windows imports");
        let certificate = imported
            .certs()
            .find(|certificate| certificate.private_key().acquire().is_ok())
            .expect("the certificate the key belongs to");
        self.add_certificate(&certificate, friendly)
    }

    fn add_certificate(&mut self, certificate: &CertContext, friendly: &str) -> CertContext {
        let mut store = CertStore::open_current_user(self.name).expect("a test store");
        let added = store
            .add_cert(certificate, CertAdd::ReplaceExisting)
            .expect("the certificate added");
        added.set_friendly_name(friendly).expect("a friendly name");
        self.added.push(
            added
                .fingerprint(HashAlgorithm::sha1())
                .expect("a fingerprint"),
        );
        added
    }

    fn naming(&self, find: StoreSearch) -> StoreCertificate {
        let mut store = StoreCertificate::new(find);
        store.name = Some(self.name.to_owned());
        store
    }
}

impl Drop for TestStore {
    fn drop(&mut self) {
        if let Ok(store) = CertStore::open_current_user(self.name) {
            delete_from(store, &self.added);
        }
    }
}

fn hex(certificate: &CertContext) -> String {
    certificate
        .fingerprint(HashAlgorithm::sha1())
        .expect("a fingerprint")
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

async fn echo(endpoint: &str, tls: TlsConfig) -> GrpcStatus {
    let mut transport = TransportConfig::new(Uri::try_from(endpoint).expect("an endpoint"));
    transport.connect_timeout = Duration::from_secs(5);
    transport.tls = tls;
    let channel = channel_with(GrpcChannelConfig::new(transport)).expect("a channel");
    let (_, _, status) = unary(
        &channel,
        CallStartOptions::new(ECHO),
        Bytes::from_static(b"hello"),
    )
    .await;
    status
}

/// The certificate the CA's root is, added to `store` as a certificate with no key.
fn root_in(store: &mut TestStore, pki: &Pki, friendly: &str) {
    let root = CertContext::new(pki.root().as_ref()).expect("the root as a certificate context");
    store.add_certificate(&root, friendly);
}

#[tokio::test]
async fn a_client_identity_and_its_root_are_found_by_each_way_of_naming_them() {
    let pki = Pki::new();
    let server = TlsServer::start(pki.server(&["127.0.0.1"]), Some(&pki)).await;
    let mut identities = TestStore::new("ArmoniKTransportTest-Identity");
    let mut roots = TestStore::new("ArmoniKTransportTest-Root");
    let added = identities.add(&pki.client(), "armonik-transport test identity", true);
    root_in(&mut roots, &pki, "armonik-transport test root");
    let thumbprint = hex(&added);

    let ways = [
        (
            "FriendlyName",
            identities.naming(StoreSearch::FriendlyName(
                "armonik-transport test identity".to_owned(),
            )),
        ),
        (
            "Thumbprint",
            identities.naming(StoreSearch::Thumbprint(thumbprint.to_uppercase())),
        ),
        (
            "SubjectName",
            identities.naming(StoreSearch::SubjectName("CLIENT.TEST".to_owned())),
        ),
    ];
    for (way, store) in ways {
        let mut options = TlsOptions::default();
        options.client = Some(ClientCertificate::Store(store));
        options.server = Some(ServerVerification::CaStore(roots.naming(
            StoreSearch::FriendlyName("armonik-transport test root".to_owned()),
        )));
        let tls = options
            .load()
            .unwrap_or_else(|refused| panic!("{way}: {refused}"));
        assert_eq!(tls.roots.len(), 1, "{way}");
        let status = echo(&server.endpoint, tls).await;
        assert_eq!(status.code, GrpcStatusCode::Ok, "{way}: {status}");
    }
}

#[tokio::test]
async fn a_key_the_store_keeps_unexportable_is_refused_by_the_option_naming_it() {
    let pki = Pki::new();
    let mut identities = TestStore::new("ArmoniKTransportTest-Unexportable");
    identities.add(&pki.client(), "armonik-transport unexportable", false);

    let mut options = TlsOptions::default();
    options.client = Some(ClientCertificate::Store(identities.naming(
        StoreSearch::FriendlyName("armonik-transport unexportable".to_owned()),
    )));
    let refused = options
        .load()
        .expect_err("a key that cannot leave the store");
    assert!(refused.key().starts_with("Client.Store."), "{refused}");
    assert!(refused.to_string().contains("without a key"), "{refused}");
}

#[tokio::test]
async fn a_certificate_with_no_key_is_refused_as_an_identity_the_same_way() {
    let pki = Pki::new();
    let mut identities = TestStore::new("ArmoniKTransportTest-Keyless");
    let keyless = CertContext::new(pki.client().chain[0].as_ref()).expect("a certificate");
    identities.add_certificate(&keyless, "armonik-transport keyless");

    let mut options = TlsOptions::default();
    options.client = Some(ClientCertificate::Store(identities.naming(
        StoreSearch::FriendlyName("armonik-transport keyless".to_owned()),
    )));
    let refused = options.load().expect_err("a certificate with no key");
    assert!(refused.to_string().contains("it has none"), "{refused}");
}

#[tokio::test]
async fn a_name_that_finds_no_certificate_or_two_is_refused() {
    let pki = Pki::new();
    let mut identities = TestStore::new("ArmoniKTransportTest-Ambiguous");
    identities.add(&pki.client(), "armonik-transport twin", true);
    identities.add(&pki.client(), "armonik-transport twin", true);

    let refusals = [
        (
            identities.naming(StoreSearch::FriendlyName(
                "armonik-transport twin".to_owned(),
            )),
            "2 certificates",
        ),
        (
            identities.naming(StoreSearch::FriendlyName("no such name".to_owned())),
            "no certificate",
        ),
        (
            identities.naming(StoreSearch::Thumbprint("not forty digits".to_owned())),
            "40 hexadecimal digits",
        ),
        (
            identities.naming(StoreSearch::SubjectName(String::new())),
            "it is empty",
        ),
    ];
    for (store, said) in refusals {
        let mut options = TlsOptions::default();
        options.client = Some(ClientCertificate::Store(store.clone()));
        let refused = options.load().expect_err("refused");
        assert!(
            refused.key().starts_with("Client.Store.Find."),
            "{store:?}: {refused}"
        );
        assert!(refused.to_string().contains(said), "{store:?}: {refused}");
    }
}

/// One certificate in the current user's own `CA` store, which issuers are read from, deleted
/// when the guard is dropped - a run killed before then leaves it there - and nothing else of
/// that store touched.
struct InCaStore(Vec<u8>);

impl InCaStore {
    fn add(der: &[u8]) -> Self {
        let certificate = CertContext::new(der).expect("a certificate context");
        let mut store = CertStore::open_current_user("CA").expect("the CA store");
        store
            .add_cert(&certificate, CertAdd::ReplaceExisting)
            .expect("the intermediate added");
        Self(
            certificate
                .fingerprint(HashAlgorithm::sha1())
                .expect("a fingerprint"),
        )
    }
}

impl Drop for InCaStore {
    fn drop(&mut self) {
        if let Ok(store) = CertStore::open_current_user("CA") {
            delete_from(store, std::slice::from_ref(&self.0));
        }
    }
}

#[tokio::test]
async fn an_identity_from_the_store_carries_the_issuers_the_ca_store_holds() {
    let root = Pki::new();
    let intermediate = root.intermediate();
    let server = TlsServer::start(root.server(&["127.0.0.1"]), Some(&root)).await;
    let client = intermediate.client();
    // A decoy first: another intermediate of the same subject, under another key, which the
    // client's certificate was not signed with.
    let _decoy = InCaStore::add(root.intermediate().root().as_ref());
    let _issuer = InCaStore::add(intermediate.root().as_ref());

    let mut identities = TestStore::new("ArmoniKTransportTest-Chain");
    let leaf_alone = Leaf {
        chain: client.chain[..1].to_vec(),
        key: client.key.clone_key(),
        chain_pem: String::new(),
        key_pem: String::new(),
    };
    identities.add(&leaf_alone, "armonik-transport chained", true);

    let mut options = TlsOptions::default();
    options.client = Some(ClientCertificate::Store(identities.naming(
        StoreSearch::FriendlyName("armonik-transport chained".to_owned()),
    )));
    let mut tls = options.load().expect("the identity in the store");
    assert_eq!(
        tls.identity.as_ref().map(|identity| identity.chain.len()),
        Some(2),
        "the leaf, then the intermediate the CA store holds"
    );
    assert_eq!(
        tls.identity
            .as_ref()
            .map(|identity| identity.chain[1].clone()),
        Some(intermediate.root()),
        "the intermediate that signed the leaf, and not the decoy"
    );
    tls.roots = vec![root.root()];
    let status = echo(&server.endpoint, tls).await;
    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
}
