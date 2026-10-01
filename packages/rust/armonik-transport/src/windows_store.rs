//! A certificate found in a Windows certificate store, for the client's identity or as a root.
//!
//! Resolved here rather than by the .NET binding, so one reader serves every host. A key leaves the
//! store the one way Windows hands a key out, a PKCS#12 export, so a key the store keeps
//! unexportable is refused when the channel is created rather than at its first handshake.

use rustls::pki_types::CertificateDer;
use schannel::cert_context::{CertContext, HashAlgorithm};
use schannel::cert_store::{CertAdd, CertStore, Memory};

/// How the one certificate is told apart from the others in its store.
pub(crate) enum By<'a> {
    /// Its SHA-1 fingerprint.
    Thumbprint([u8; 20]),
    /// A text one of its subject's attribute values contains, compared without case, as
    /// .NET's `FindBySubjectName` does.
    SubjectName(&'a str),
    /// Its friendly name, exactly.
    FriendlyName(&'a str),
}

/// The password a bundle is exported under. It never leaves this process: the bytes are read back
/// at once, and the key they carry is what the identity holds anyway.
pub(crate) const EXPORT_PASSWORD: &str = "armonik-transport";

/// How far the issuers above a certificate are followed, which ends a loop the store could hold.
const LONGEST_CHAIN: usize = 8;

fn open(local_machine: bool, name: &str) -> Result<CertStore, String> {
    let opened = if local_machine {
        CertStore::open_local_machine(name)
    } else {
        CertStore::open_current_user(name)
    };
    opened.map_err(|error| format!("the store `{name}` could not be opened: {error}"))
}

/// The subject's attribute values, without the names that label them: a text a search names
/// is looked for in what the subject says, not in how it is spelled.
fn subject_values(certificate: &CertContext) -> Vec<String> {
    let Ok((_, parsed)) = x509_parser::parse_x509_certificate(certificate.to_der()) else {
        return Vec::new();
    };
    parsed
        .subject()
        .iter_attributes()
        .filter_map(|attribute| attribute.as_str().ok())
        .map(str::to_lowercase)
        .collect()
}

fn matches(certificate: &CertContext, by: &By<'_>) -> bool {
    match by {
        By::Thumbprint(thumbprint) => certificate
            .fingerprint(HashAlgorithm::sha1())
            .is_ok_and(|sha1| sha1 == thumbprint),
        By::FriendlyName(name) => certificate
            .friendly_name()
            .is_ok_and(|friendly| friendly == *name),
        By::SubjectName(name) => {
            let name = name.to_lowercase();
            subject_values(certificate)
                .iter()
                .any(|value| value.contains(&name))
        }
    }
}

/// The one certificate of the store `name` that `by` matches.
pub(crate) fn find(local_machine: bool, name: &str, by: &By<'_>) -> Result<CertContext, String> {
    let store = open(local_machine, name)?;
    let mut found: Vec<CertContext> = store
        .certs()
        .filter(|certificate| matches(certificate, by))
        .collect();
    match found.len() {
        0 => Err(format!(
            "no certificate of the store `{name}` matches it; a name no store had opens an empty one"
        )),
        1 => Ok(found.remove(0)),
        many => Err(format!(
            "{many} certificates of the store `{name}` match it, and nothing says which to use"
        )),
    }
}

/// The certificate and its key, as a PKCS#12 bundle protected by [`EXPORT_PASSWORD`].
pub(crate) fn export(certificate: &CertContext) -> Result<Vec<u8>, String> {
    let mut store = Memory::new()
        .map_err(|error| format!("no store could be made to export from: {error}"))?
        .into_store();
    store
        .add_cert(certificate, CertAdd::Always)
        .map_err(|error| format!("the certificate could not be copied to export: {error}"))?;
    store.export_pkcs12(EXPORT_PASSWORD).map_err(|error| {
        format!("its key could not be exported, as a key the store keeps unexportable cannot be: {error}")
    })
}

/// The issuers above `certificate` that the `CA` store of the same location holds, nearest
/// first, up to but not including a self-signed root, which the server holds itself.
///
/// An issuer is a certificate whose subject is the issuer's name and whose key signed the one
/// below it, a time-valid one first when the store holds several. A store that cannot be opened
/// gives no issuer, as one holding none does; the handshake is then what reports a short chain.
pub(crate) fn issuers(
    certificate: &CertContext,
    local_machine: bool,
) -> Vec<CertificateDer<'static>> {
    let Ok(store) = open(local_machine, "CA") else {
        return Vec::new();
    };
    let candidates: Vec<CertContext> = store.certs().collect();
    let mut chain = Vec::new();
    let mut current = certificate.to_der().to_vec();
    while chain.len() < LONGEST_CHAIN {
        let Ok((_, parsed)) = x509_parser::parse_x509_certificate(&current) else {
            break;
        };
        let issuer = parsed.issuer().as_raw();
        if issuer == parsed.subject().as_raw() {
            break;
        }
        let named: Vec<&CertContext> = candidates
            .iter()
            .filter(|candidate| {
                x509_parser::parse_x509_certificate(candidate.to_der()).is_ok_and(|(_, above)| {
                    above.subject().as_raw() == issuer
                        && above.subject().as_raw() != above.issuer().as_raw()
                        && parsed.verify_signature(Some(above.public_key())).is_ok()
                })
            })
            .collect();
        let above = named
            .iter()
            .find(|candidate| candidate.is_time_valid().unwrap_or(false))
            .or_else(|| named.first());
        let Some(above) = above else {
            break;
        };
        current = above.to_der().to_vec();
        chain.push(CertificateDer::from(current.clone()));
    }
    chain
}
