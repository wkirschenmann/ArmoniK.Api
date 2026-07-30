//! Fixtures shared by this crate's unit tests.

/// A self-signed certificate and its key, PEM-armoured.
///
/// Real `rcgen` output rather than a hand-written fixture, so the code under test sees exactly what a
/// real certificate looks like — `rustls`'s own PEM reader is part of what is being exercised. Each
/// call generates a fresh, independent pair, which is what lets a test build a *mismatched* identity
/// out of two of them.
pub(crate) fn certificate() -> (String, String) {
    let params =
        rcgen::CertificateParams::new(vec!["localhost".to_owned()]).expect("certificate params");
    let key_pair = rcgen::KeyPair::generate().expect("key pair");
    let cert = params
        .self_signed(&key_pair)
        .expect("self-signed certificate");
    (cert.pem(), key_pair.serialize_pem())
}
