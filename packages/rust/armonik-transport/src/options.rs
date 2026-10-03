//! The options a caller sets on a channel, as a document carries them.
//!
//! Structured and typed, because the schema derived from these types is what generates the
//! options class a .NET caller fills in: a number is a number, a group of options is an object,
//! and every constraint that can be said here is said here rather than only in the code that
//! enforces it. What a type cannot say - that an endpoint names a scheme this engine speaks -
//! the transport says, by option name.

use std::time::Duration;

use hyper::Uri;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use secrecy::ExposeSecret;

use crate::grpc::RetryConfig;
use crate::http2::{ClientIdentity, Http2Config, ProxyConfig, ProxySource, TcpConfig, TlsConfig};

/// The largest window either side of a call may be given.
///
/// A window becomes a `tokio` semaphore, which refuses more than `Semaphore::MAX_PERMITS`
/// permits, and that limit is `usize::MAX >> 3` - so it is far larger on a 64-bit target than on
/// a 32-bit one. The schema is one file for every target, so the bound it states is the tighter
/// of the two; `a_window_the_schema_admits_is_one_a_semaphore_admits` is what keeps that true.
pub const LARGEST_WINDOW: i32 = 536_870_910;

/// A duration, in seconds.
///
/// Seconds rather than a `Duration`, whose schema is `{ secs, nanos }` - this crate's memory
/// layout rather than anything a document would write. A number carries no unit, so every
/// option of this type names one: `connect_timeout_seconds`, not `connect_timeout`.
///
/// The ceiling is the type's own and not any one option's: every `Seconds` becomes a `Duration`,
/// and `Duration` holds `u64::MAX` seconds, so 2^64 is the first value none can be. Stated here
/// rather than left to the conversion, which refuses correctly but names no option when it does -
/// a caller then reads that their configuration was refused and not which line of it.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(extend("exclusiveMaximum" = 18446744073709551616.0))
)]
pub struct Seconds(pub f64);

impl TryFrom<Seconds> for Duration {
    type Error = std::time::TryFromFloatSecsError;

    /// Fallible because a document names a number and not every number is a duration: a
    /// `Duration` holds neither a negative value nor one past its own range, and both are
    /// ordinary doubles a caller is free to write.
    fn try_from(value: Seconds) -> Result<Self, Self::Error> {
        Duration::try_from_secs_f64(value.0)
    }
}

/// What the transport does, beyond reaching the endpoint it was given.
///
/// The endpoint is not here: it is the one value a channel cannot be created without, so it is
/// passed when the channel is opened rather than set as an option that happens to be mandatory.
/// Every option has a default, so naming none of them is a valid configuration.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(rename_all = "PascalCase", deny_unknown_fields)
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct TransportOptions {
    /// How long a dial may take before it is given up on.
    ///
    /// Defaults to 60, and at least a nanosecond, the finest duration the engine holds: a shorter
    /// one could round to zero, which no dial could beat.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    // Set beside the `$ref` that `with` writes, where `range` does not reach.
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Seconds", extend("minimum" = 1e-9))
    )]
    pub connect_timeout_seconds: Option<Seconds>,

    /// How an `https://` endpoint is secured.
    ///
    /// Defaults to `{}`: the server verified against the system's roots under the endpoint's
    /// host, and no client certificate. Refused for an `http://` endpoint unless it sets nothing.
    #[cfg_attr(feature = "serde", serde(default))]
    pub tls: TlsOptions,

    /// The socket's keepalive.
    ///
    /// Defaults to `{}`, which sets none.
    #[cfg_attr(feature = "serde", serde(default))]
    pub tcp_keepalive: TcpKeepaliveOptions,

    /// The HTTP proxy every dial tunnels through.
    ///
    /// Defaults to `{}`, which tunnels through the proxy the environment names, if any.
    #[cfg_attr(feature = "serde", serde(default))]
    pub proxy: ProxyOptions,
}

/// An HTTP proxy, which a dial tunnels through with `CONNECT`, so TLS stays end to end with the
/// server.
#[derive(Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(rename_all = "PascalCase", deny_unknown_fields)
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct ProxyOptions {
    /// `none` for no proxy, `system` for the one the system names, or the proxy's `http://` URL,
    /// with no path; `http://` is assumed when no scheme is written. The URL may carry `user:password@`,
    /// percent-encoded, when `Username` and `Password` are not set - which a serialized document
    /// then carries too.
    ///
    /// The environment's proxy is `ALL_PROXY`, `HTTPS_PROXY` or `HTTP_PROXY`, in either case and
    /// by the endpoint's scheme, unless `NO_PROXY` names the endpoint's host; it is read when the
    /// channel is created. An `https://` or `socks` one is refused when the channel is created,
    /// and any other value the environment cannot read as a proxy is ignored.
    ///
    /// On Windows, when the environment names no proxy, the system's is the one the current
    /// user's network settings name: a PAC script, detected or at the configured address, which
    /// WinHTTP fetches and runs for each dial off the calling thread, else the manual proxy and
    /// its bypass list. Those settings are read when the channel is created, and an `https://` or
    /// `socks` proxy they name is refused at each dial - except a script's `SOCKS` answer, which
    /// WinHTTP drops, leaving a direct dial. A script that cannot be found or run is not tried
    /// again for two minutes.
    ///
    /// The system's proxy is never used for a loopback endpoint.
    ///
    /// Defaults to `system`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "String", length(min = 1)))]
    pub address: Option<String>,

    /// The username the proxy is authenticated to with, by `Basic`, which forbids a `:` in it.
    ///
    /// Refused beside credentials the `Address` URL carries; ignored beside `none`, and when the
    /// system names no proxy. Beside the environment's proxy, it takes the place of the username
    /// that proxy's URL carries; beside the one Windows' settings name, it is the username.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub username: Option<String>,

    /// The password that goes with `Username`.
    ///
    /// Refused beside credentials the `Address` URL carries; ignored beside `none`, and when the
    /// system names no proxy. Beside the environment's proxy, it takes the place of the password
    /// that proxy's URL carries; beside the one Windows' settings name, it is the password.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing))]
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub password: Option<Password>,
}

/// The address as a Debug print may show it, which is how a URL is typed and not how it parses:
/// what precedes the last `@` goes, and so does what follows a `:` that is not a port, which is
/// how `user:password` reads when its `@host` was left out.
fn elided(address: &str) -> String {
    let (scheme, rest) = match address.split_once("://") {
        Some((scheme, rest)) => (format!("{scheme}://"), rest),
        None => (String::new(), address),
    };
    if let Some((_, host)) = rest.rsplit_once('@') {
        return format!("{scheme}***@{host}");
    }
    // A bracketed host's colons are its own; the port, if any, follows the bracket.
    let (host, after) = match rest
        .strip_prefix('[')
        .and_then(|inner| inner.split_once(']'))
    {
        Some((inner, after)) => (format!("[{inner}]"), after),
        None => match rest.split_once(':') {
            Some((host, _)) => (host.to_owned(), &rest[host.len()..]),
            None => return address.to_owned(),
        },
    };
    match after.strip_prefix(':') {
        Some(tail) if tail.trim_end_matches('/').parse::<u16>().is_err() => {
            format!("{scheme}{host}:***")
        }
        _ => address.to_owned(),
    }
}

/// The address is printed elided, since it may carry a password.
impl std::fmt::Debug for ProxyOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyOptions")
            .field("address", &self.address.as_deref().map(elided))
            .field("username", &self.username)
            .field("password", &self.password)
            .finish()
    }
}

impl ProxyOptions {
    /// The proxy these options name. A refusal never quotes the address, which may hold a
    /// password.
    pub fn to_config(&self) -> Result<ProxyConfig, OptionRefusal> {
        let dedicated = self.username.is_some() || self.password.is_some();
        let address = match self.address.as_deref() {
            None => return self.with_dedicated(ProxySource::System),
            // Credentials beside `none` are left unread: a deployment that turned its proxy off
            // without clearing the credentials it needed is in a normal state.
            Some(none) if none.eq_ignore_ascii_case("none") => return Ok(ProxyConfig::default()),
            Some(system) if system.eq_ignore_ascii_case("system") => {
                return self.with_dedicated(ProxySource::System)
            }
            Some(address) => address,
        };

        let not_a_url = || {
            OptionRefusal::new(
                "Address",
                "it is neither `none` nor a proxy URL such as `http://proxy.example.com:3128`",
            )
        };
        let written = if address.contains("://") {
            address.to_owned()
        } else {
            format!("http://{address}")
        };
        let uri: Uri = written.parse().map_err(|_| not_a_url())?;
        if uri.scheme_str() != Some("http") {
            return Err(OptionRefusal::new(
                "Address",
                "it has to be an `http://` URL: the `CONNECT` handshake is written in the clear",
            ));
        }
        let authority = uri.authority().ok_or_else(not_a_url)?.as_str();
        // The last `@`, because a password may hold one.
        let (userinfo, host) = match authority.rsplit_once('@') {
            Some((userinfo, host)) => (Some(userinfo), host),
            None => (None, authority),
        };
        // A port that does not parse would otherwise be dialled as 80; a bracketed host's own
        // colons stay inside its brackets.
        let port = host
            .rsplit_once(':')
            .map(|(_, port)| port)
            .filter(|port| !port.contains(']'));
        if host.is_empty()
            || host.starts_with(':')
            || port.is_some_and(|port| port.parse::<u16>().map_or(true, |port| port == 0))
        {
            return Err(not_a_url());
        }
        if !matches!(uri.path(), "" | "/") || uri.query().is_some() || written.contains('#') {
            return Err(OptionRefusal::new(
                "Address",
                "it carries a path, a query or a fragment, which a proxy is not addressed by",
            ));
        }
        if userinfo.is_some() && dedicated {
            return Err(OptionRefusal::new(
                "Address",
                "it carries `user:password@`, and so do Username or Password; set them one way",
            ));
        }
        let proxy = Uri::builder()
            .scheme("http")
            .authority(host)
            .path_and_query("/")
            .build()
            .map_err(|_| not_a_url())?;

        // Strict: a byte that is not UTF-8 would otherwise become a replacement character, and a
        // password the user did not write.
        let decoded = |text: &str| -> Result<String, OptionRefusal> {
            percent_encoding::percent_decode_str(text)
                .decode_utf8()
                .map(|text| text.into_owned())
                .map_err(|_| not_a_url())
        };
        let source = ProxySource::Explicit(proxy);
        match userinfo {
            Some(userinfo) => {
                let (username, password) = match userinfo.split_once(':') {
                    Some((username, password)) => (decoded(username)?, decoded(password)?),
                    None => (decoded(userinfo)?, String::new()),
                };
                if username.contains(':') {
                    return Err(OptionRefusal::new("Address", NO_COLON));
                }
                Ok(ProxyConfig {
                    source,
                    username,
                    password: password.into(),
                })
            }
            None => self.with_dedicated(source),
        }
    }

    /// `source`, authenticated to with `Username` and `Password`, empty when unset.
    fn with_dedicated(&self, source: ProxySource) -> Result<ProxyConfig, OptionRefusal> {
        let username = self.username.clone().unwrap_or_default();
        if username.contains(':') {
            return Err(OptionRefusal::new("Username", NO_COLON));
        }
        let password = self
            .password
            .as_ref()
            .map(|password| password.0.expose_secret().to_owned())
            .unwrap_or_default();
        Ok(ProxyConfig {
            source,
            username,
            password: password.into(),
        })
    }
}

/// `Basic` splits user and password at the first `:`, so one in the user moves the rest into the
/// password.
const NO_COLON: &str = "the username holds a `:`, which `Basic` authentication cannot carry";

/// How an `https://` endpoint is secured. Each file is read when the channel is created, so a
/// path that names nothing usable is refused then, by its option's name.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(rename_all = "PascalCase", deny_unknown_fields)
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct TlsOptions {
    /// Path to a PEM file of the roots the server certificate is verified against, in place of
    /// the system's. Every certificate the file holds is a root.
    ///
    /// Refused together with `AllowUnsafeConnection`, which verifies nothing.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "String", length(min = 1)))]
    pub ca_cert_path: Option<String>,

    /// Path to a PEM file of the client's certificate, then each issuer the server may not hold.
    ///
    /// Set together with `KeyPem`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "String", length(min = 1)))]
    pub cert_pem: Option<String>,

    /// Path to a PEM file of the key of the client's certificate.
    ///
    /// Set together with `CertPem`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "String", length(min = 1)))]
    pub key_pem: Option<String>,

    /// Path to a PKCS#12 bundle of the client's certificate, the issuers it carries and the key.
    ///
    /// Refused together with `CertPem` or `KeyPem`, which name an identity too.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "String", length(min = 1)))]
    pub cert_p12: Option<String>,

    /// The password `CertP12` is protected by. Defaults to the empty one.
    ///
    /// Refused without `CertP12`.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing))]
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub cert_p12_password: Option<Password>,

    /// The client's certificate and key from a Windows certificate store, `My` unless `Name`
    /// says otherwise, with the issuers the store's `CA` holds. Its key has to be exportable.
    ///
    /// Refused together with `CertPem`, `KeyPem` or `CertP12`, and off Windows.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "StoreCertificate"))]
    pub cert_store: Option<StoreCertificate>,

    /// The root the server certificate is verified against, from a Windows certificate store,
    /// `Root` unless `Name` says otherwise, in place of the system's.
    ///
    /// Refused together with `CaCertPath` or `AllowUnsafeConnection`, and off Windows.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "StoreCertificate"))]
    pub ca_store: Option<StoreCertificate>,

    /// Accept any server certificate. The connection is still encrypted, to whoever answers.
    ///
    /// Defaults to false.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "bool"))]
    pub allow_unsafe_connection: Option<bool>,

    /// The host the server certificate is verified against, and sent as SNI, in place of the
    /// endpoint's: a DNS name or an IP address, `[::1]` for IPv6, with an optional port that is
    /// not read.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "String", length(min = 1)))]
    pub override_target_name: Option<String>,
}

/// A certificate of a Windows certificate store, named by exactly one of `Thumbprint`,
/// `SubjectName` and `FriendlyName`.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(rename_all = "PascalCase", deny_unknown_fields)
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct StoreCertificate {
    /// `CurrentUser` or `LocalMachine`.
    ///
    /// Defaults to `CurrentUser`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "String", length(min = 1)))]
    pub location: Option<String>,

    /// The store's name, such as `My`, `Root` or `CA`. Defaults to the one its option states.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "String", length(min = 1)))]
    pub name: Option<String>,

    /// The certificate's SHA-1 fingerprint, as 40 hexadecimal digits; spaces and colons between
    /// them are ignored.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "String", length(min = 1)))]
    pub thumbprint: Option<String>,

    /// A text the certificate's subject contains, compared without case, as .NET's
    /// `FindBySubjectName` compares it.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "String", length(min = 1)))]
    pub subject_name: Option<String>,

    /// The certificate's friendly name, exactly.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "String", length(min = 1)))]
    pub friendly_name: Option<String>,
}

/// A thumbprint as the 20 bytes it writes, with what a copy from a certificate dialog carries
/// around them - spaces, colons, and the left-to-right mark - taken out.
#[cfg_attr(not(windows), allow(dead_code))]
fn thumbprint(written: &str) -> Result<[u8; 20], OptionRefusal> {
    let digits: String = written
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ':' && *c != '\u{200e}')
        .collect();
    if digits.len() != 40 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(OptionRefusal::new(
            "Thumbprint",
            "it has to be 40 hexadecimal digits, a SHA-1 fingerprint",
        ));
    }
    let mut bytes = [0u8; 20];
    for (byte, pair) in bytes.iter_mut().zip(digits.as_bytes().chunks(2)) {
        let pair = std::str::from_utf8(pair).expect("ASCII digits");
        *byte = u8::from_str_radix(pair, 16).expect("hexadecimal digits");
    }
    Ok(bytes)
}

impl StoreCertificate {
    /// The store this names, `default` unless `Name` is set, and where it sits.
    #[cfg_attr(not(windows), allow(dead_code))]
    fn place<'a>(&'a self, default: &'a str) -> Result<(bool, &'a str), OptionRefusal> {
        // An empty text would match every certificate's subject, and pick one silently.
        for (key, text) in [
            ("Location", &self.location),
            ("Name", &self.name),
            ("SubjectName", &self.subject_name),
            ("FriendlyName", &self.friendly_name),
        ] {
            if text.as_deref() == Some("") {
                return Err(OptionRefusal::new(key, "it is empty"));
            }
        }
        let local_machine = match self.location.as_deref() {
            None | Some("CurrentUser") => false,
            Some("LocalMachine") => true,
            Some(other) => {
                return Err(OptionRefusal::new(
                    "Location",
                    format!("`{other}` is neither CurrentUser nor LocalMachine"),
                ))
            }
        };
        Ok((local_machine, self.name.as_deref().unwrap_or(default)))
    }

    /// The one certificate this names, in the store `default` unless `Name` is set, where the
    /// store sits, and the option that named it, which a later refusal is reported against.
    #[cfg(windows)]
    fn find(
        &self,
        default: &str,
    ) -> Result<(schannel::cert_context::CertContext, bool, &'static str), OptionRefusal> {
        use crate::windows_store::By;

        let (local_machine, name) = self.place(default)?;
        let by =
            match (&self.thumbprint, &self.subject_name, &self.friendly_name) {
                (Some(written), None, None) => By::Thumbprint(thumbprint(written)?),
                (None, Some(subject), None) => By::SubjectName(subject),
                (None, None, Some(friendly)) => By::FriendlyName(friendly),
                (None, None, None) => return Err(OptionRefusal::new(
                    "Thumbprint",
                    "one of Thumbprint, SubjectName and FriendlyName has to name the certificate",
                )),
                _ => return Err(OptionRefusal::new(
                    "Thumbprint",
                    "only one of Thumbprint, SubjectName and FriendlyName may name the certificate",
                )),
            };
        let key = match by {
            By::Thumbprint(_) => "Thumbprint",
            By::SubjectName(_) => "SubjectName",
            By::FriendlyName(_) => "FriendlyName",
        };
        crate::windows_store::find(local_machine, name, &by)
            .map(|found| (found, local_machine, key))
            .map_err(|why| OptionRefusal::new(key, why))
    }

    /// The client identity this names: the certificate, its key, and its issuers.
    #[cfg(windows)]
    fn identity(&self) -> Result<ClientIdentity, OptionRefusal> {
        let (certificate, local_machine, named) = self.find("My")?;
        let bundle = crate::windows_store::export(&certificate)
            .map_err(|why| OptionRefusal::new(named, why))?;
        // Windows exports a certificate without its key, and without an error, when the key is
        // not one it lets out, so the absence is all there is to report.
        let mut identity = open_pkcs12(&bundle, crate::windows_store::EXPORT_PASSWORD).map_err(
            |why| match why {
                Unopened::NoKey => OptionRefusal::new(
                    named,
                    "the certificate it names leaves the store without a key: it has none, or \
                     the store keeps its key unexportable - a TPM, a smart card, or an import \
                     without the exportable flag - and only a key the store lets out can be used",
                ),
                why => OptionRefusal::new(
                    named,
                    format!("the bundle its certificate exports to {why}"),
                ),
            },
        )?;
        identity.chain.truncate(1);
        identity
            .chain
            .extend(crate::windows_store::issuers(&certificate, local_machine));
        Ok(identity)
    }

    /// The root this names.
    #[cfg(windows)]
    fn root(&self) -> Result<CertificateDer<'static>, OptionRefusal> {
        let (certificate, _, _) = self.find("Root")?;
        Ok(CertificateDer::from(certificate.to_der().to_vec()))
    }

    #[cfg(not(windows))]
    fn identity(&self) -> Result<ClientIdentity, OptionRefusal> {
        Err(OptionRefusal::new(
            "Name",
            "a Windows certificate store exists on Windows only",
        ))
    }

    #[cfg(not(windows))]
    fn root(&self) -> Result<CertificateDer<'static>, OptionRefusal> {
        Err(OptionRefusal::new(
            "Name",
            "a Windows certificate store exists on Windows only",
        ))
    }
}

/// A password: no Debug print shows it, no message quotes it, and it is zeroed when dropped.
#[derive(Clone)]
pub struct Password(secrecy::SecretString);

/// Read by hand, so that a value of the wrong type is refused without being quoted: serde's own
/// refusal names the value it was given, and a refusal is shown to whoever configured it.
#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Password {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Text;

        fn refused<E: serde::de::Error>() -> E {
            E::custom("a password has to be a string")
        }

        impl<'de> serde::de::Visitor<'de> for Text {
            type Value = Password;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a string")
            }

            fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<Password, E> {
                Ok(Password::new(text))
            }

            fn visit_string<E: serde::de::Error>(self, text: String) -> Result<Password, E> {
                Ok(Password::new(text))
            }

            fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Password, E> {
                Err(refused())
            }

            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Password, E> {
                Err(refused())
            }

            fn visit_i128<E: serde::de::Error>(self, _: i128) -> Result<Password, E> {
                Err(refused())
            }

            fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Password, E> {
                Err(refused())
            }

            fn visit_u128<E: serde::de::Error>(self, _: u128) -> Result<Password, E> {
                Err(refused())
            }

            fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Password, E> {
                Err(refused())
            }

            fn visit_bytes<E: serde::de::Error>(self, _: &[u8]) -> Result<Password, E> {
                Err(refused())
            }

            fn visit_unit<E: serde::de::Error>(self) -> Result<Password, E> {
                Err(refused())
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, _: A) -> Result<Password, A::Error> {
                Err(refused())
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(self, _: A) -> Result<Password, A::Error> {
                Err(refused())
            }
        }

        // `any` rather than `string`: a format asked for a string refuses another type itself,
        // quoting it, before the visitor is reached.
        deserializer.deserialize_any(Text)
    }
}

impl Password {
    pub fn new(text: impl Into<String>) -> Self {
        Self(secrecy::SecretString::from(text.into()))
    }
}

impl PartialEq for Password {
    fn eq(&self, other: &Self) -> bool {
        self.0.expose_secret() == other.0.expose_secret()
    }
}

impl std::fmt::Debug for Password {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Password(..)")
    }
}

/// The socket's keepalive, off unless `IdleSeconds` is set.
///
/// Each duration is whole seconds, which is what the socket option holds: a fraction is dropped.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(rename_all = "PascalCase", deny_unknown_fields)
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct TcpKeepaliveOptions {
    /// How long the connection may be idle before the first probe, from a second to 32767, the
    /// most Linux holds.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Seconds", extend("minimum" = 1.0, "maximum" = 32767.0))
    )]
    pub idle_seconds: Option<Seconds>,

    /// How long between two probes, from a second to 32767. Defaults to the operating system's.
    ///
    /// Refused without `IdleSeconds`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Seconds", extend("minimum" = 1.0, "maximum" = 32767.0))
    )]
    pub interval_seconds: Option<Seconds>,

    /// How many probes go unanswered before the connection is dropped, at most 127, the most
    /// Linux holds. Defaults to the operating system's, and is not applied on Windows.
    ///
    /// Refused without `IdleSeconds`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "i32", range(min = 1, max = 127)))]
    pub retries: Option<i32>,
}

/// The HTTP/2 session a channel's calls share: how it checks that the peer is there, and how much
/// it lets the peer send ahead of what is read.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(rename_all = "PascalCase", deny_unknown_fields)
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct Http2Options {
    /// How often a PING is sent to the peer. Defaults to none sent.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Seconds", extend("minimum" = 1e-9))
    )]
    pub keep_alive_interval_seconds: Option<Seconds>,

    /// How long a PING may go unanswered before the session and its calls are ended.
    ///
    /// Defaults to 20.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Seconds", extend("minimum" = 1e-9))
    )]
    pub keep_alive_timeout_seconds: Option<Seconds>,

    /// Whether a PING is also sent while no call is open.
    ///
    /// Defaults to false.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "bool"))]
    pub keep_alive_while_idle: Option<bool>,

    /// How many bytes of one call the peer may send ahead of what is read.
    ///
    /// Defaults to 2097152, 2 MiB.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "i32", range(min = 1)))]
    pub stream_window_size: Option<i32>,

    /// How many bytes the peer may send ahead of what is read, across every call of the channel.
    /// A call its host does not read holds up to `StreamWindowSize` of it, so enough of them stop
    /// the others receiving. At least 65535, the window every connection starts with.
    ///
    /// Defaults to 5242880, 5 MiB.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "i32", range(min = 65535)))]
    pub connection_window_size: Option<i32>,

    /// How long the session stays open with no call on it before it is closed, the next call
    /// dialling a new one. A call holds the session from its dial to the end of its response.
    ///
    /// Defaults to none: an idle session stays open.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Seconds", extend("minimum" = 1e-9))
    )]
    pub idle_timeout_seconds: Option<Seconds>,

    /// How many bytes a write to the connection may gather before it goes. A write waits while
    /// the work already ready adds frames to it, one round of the runtime at a time, and goes once
    /// a round adds none or it holds this many bytes: a request's message handed over while its
    /// headers wait then goes out with them, in one write rather than two. 0 writes at once.
    ///
    /// Defaults to 16384, 16 KiB.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "i32", range(min = 0)))]
    pub write_coalescing_bytes: Option<i32>,
}

/// When a failed call is sent again, as gRFC A6 has it: after a backoff drawn below a bound
/// that starts at `InitialBackoffSeconds` and grows by `BackoffMultiplier` to
/// `MaxBackoffSeconds`, for UNAVAILABLE, ABORTED and UNKNOWN, while no response head has reached
/// the reader and what the call sent is still kept for the replay.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(rename_all = "PascalCase", deny_unknown_fields)
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct RetryOptions {
    /// Attempts in all, the first included; 1 retries nothing. A call its peer never processed
    /// goes again besides, whatever this is, while every message it sent is kept.
    ///
    /// Defaults to 5.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "i32", range(min = 1)))]
    pub max_attempts: Option<i32>,

    /// The bound of the first backoff.
    ///
    /// Defaults to 1.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Seconds", extend("minimum" = 1e-9))
    )]
    pub initial_backoff_seconds: Option<Seconds>,

    /// What the bound grows to and no further; refused below `InitialBackoffSeconds`.
    ///
    /// Defaults to 5.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Seconds", extend("minimum" = 1e-9))
    )]
    pub max_backoff_seconds: Option<Seconds>,

    /// What each bound is multiplied by; 1 retries at a fixed bound.
    ///
    /// Defaults to 1.5.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "f64", extend("minimum" = 1.0)))]
    pub backoff_multiplier: Option<f64>,

    /// The bytes one call may keep for a replay; a call that sends more is not tried again.
    ///
    /// Defaults to 1048576, 1 MiB.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "i32", range(min = 0)))]
    pub call_replay_bytes: Option<i32>,

    /// The bytes all of the channel's calls may keep for a replay together; a call whose message
    /// would pass it is not tried again.
    ///
    /// Defaults to 16777216, 16 MiB.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "i32", range(min = 0)))]
    pub channel_replay_bytes: Option<i32>,
}

impl RetryOptions {
    /// The policy these options name, each unset one at its default.
    pub fn to_config(&self) -> Result<RetryConfig, OptionRefusal> {
        let defaults = RetryConfig::default();
        let count = |key: &str, asked: Option<i32>, least: i32, default: usize| match asked {
            None => Ok(default),
            Some(value) if value < least => Err(OptionRefusal::new(
                key,
                format!("{value} has to be at least {least}"),
            )),
            Some(value) => Ok(value as usize),
        };
        let initial_backoff = duration(
            "InitialBackoffSeconds",
            self.initial_backoff_seconds,
            1e-9,
            None,
        )?
        .unwrap_or(defaults.initial_backoff);
        let max_backoff = duration("MaxBackoffSeconds", self.max_backoff_seconds, 1e-9, None)?
            .unwrap_or(defaults.max_backoff);
        if max_backoff < initial_backoff {
            return Err(OptionRefusal::new(
                "MaxBackoffSeconds",
                "it is below InitialBackoffSeconds, the bound the backoff starts from",
            ));
        }
        let backoff_multiplier = match self.backoff_multiplier {
            None => defaults.backoff_multiplier,
            Some(value) if value.is_finite() && value >= 1.0 => value,
            Some(value) => {
                return Err(OptionRefusal::new(
                    "BackoffMultiplier",
                    format!("{value} has to be a finite number of at least 1"),
                ))
            }
        };
        Ok(RetryConfig {
            max_attempts: count(
                "MaxAttempts",
                self.max_attempts,
                1,
                defaults.max_attempts as usize,
            )? as u32,
            initial_backoff,
            max_backoff,
            backoff_multiplier,
            call_replay_bytes: count(
                "CallReplayBytes",
                self.call_replay_bytes,
                0,
                defaults.call_replay_bytes,
            )?,
            channel_replay_bytes: count(
                "ChannelReplayBytes",
                self.channel_replay_bytes,
                0,
                defaults.channel_replay_bytes,
            )?,
            ..defaults
        })
    }
}

/// An option refused, named by its path from the unit that read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionRefusal {
    key: String,
    why: String,
}

impl OptionRefusal {
    fn new(key: &str, why: impl Into<String>) -> Self {
        Self {
            key: key.to_owned(),
            why: why.into(),
        }
    }

    /// The same refusal, named from the unit `unit` sits in: a unit does not know where it is
    /// embedded, so the embedding adds its own name.
    pub fn under(self, unit: &str) -> Self {
        Self {
            key: format!("{unit}.{}", self.key),
            why: self.why,
        }
    }

    pub fn key(&self) -> &str {
        &self.key
    }
}

impl std::fmt::Display for OptionRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} is refused: {}", self.key, self.why)
    }
}

impl std::error::Error for OptionRefusal {}

/// A number of seconds as a duration, refused below `least`, above `most`, and past what a
/// `Duration` holds.
fn duration(
    key: &str,
    seconds: Option<Seconds>,
    least: f64,
    most: Option<f64>,
) -> Result<Option<Duration>, OptionRefusal> {
    let Some(seconds) = seconds else {
        return Ok(None);
    };
    let refused = || {
        let most = most.map_or_else(
            || "less than 2^64".to_owned(),
            |most| format!("at most {most}"),
        );
        OptionRefusal::new(
            key,
            format!("{} has to be at least {least} and {most}", seconds.0),
        )
    };
    if seconds.0 < least || most.is_some_and(|most| seconds.0 > most) {
        return Err(refused());
    }
    Duration::try_from(seconds).map(Some).map_err(|_| refused())
}

/// The bytes of a file a path option names.
///
/// The path is not repeated: the option's name says which file, and a key's path is one of the
/// things a message must not carry.
fn read(key: &str, path: &str) -> Result<Vec<u8>, OptionRefusal> {
    std::fs::read(path).map_err(|error| {
        OptionRefusal::new(key, format!("the file it names could not be read: {error}"))
    })
}

/// Every certificate of a PEM file, in the order the file writes them.
fn certificates(key: &str, path: &str) -> Result<Vec<CertificateDer<'static>>, OptionRefusal> {
    let pem = read(key, path)?;
    let certificates = CertificateDer::pem_slice_iter(&pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            OptionRefusal::new(key, format!("the file it names is not PEM: {error}"))
        })?;
    if certificates.is_empty() {
        return Err(OptionRefusal::new(
            key,
            "the file it names holds no certificate",
        ));
    }
    Ok(certificates)
}

/// The identity a PKCS#12 bundle carries: the certificate its key belongs to, then each issuer
/// it holds, in the leaf-first order rustls sends.
fn pkcs12(
    key: &str,
    path: &str,
    password: Option<&Password>,
) -> Result<ClientIdentity, OptionRefusal> {
    let bundle = read(key, path)?;
    let password = password.map_or("", |password| password.0.expose_secret());
    open_pkcs12(&bundle, password)
        .map_err(|why| OptionRefusal::new(key, format!("the bundle it names {why}")))
}

/// Why a bundle carries no identity; it reads after "the bundle".
enum Unopened {
    Refused(String),
    Keys(usize),
    NoKey,
}

impl std::fmt::Display for Unopened {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(error) => write!(f, "could not be opened: {error}"),
            Self::Keys(many) => write!(f, "holds {many} keys, and nothing says which to use"),
            Self::NoKey => f.write_str("holds no key and certificate"),
        }
    }
}

/// The identity `bundle` carries.
fn open_pkcs12(bundle: &[u8], password: &str) -> Result<ClientIdentity, Unopened> {
    // Strict: a bundle whose chain cannot be rebuilt is a mistake to report, not one to paper
    // over with part of an identity.
    let store = p12_keystore::KeyStore::from_pkcs12(
        bundle,
        password,
        p12_keystore::Pkcs12ImportPolicy::Strict,
    )
    .map_err(|error| Unopened::Refused(error.to_string()))?;
    let identities = store
        .entries()
        .filter(|(_, entry)| matches!(entry, p12_keystore::KeyStoreEntry::PrivateKeyChain(_)))
        .count();
    if identities > 1 {
        return Err(Unopened::Keys(identities));
    }
    let Some((_, chain)) = store.private_key_chain() else {
        return Err(Unopened::NoKey);
    };
    Ok(ClientIdentity {
        chain: chain
            .certs()
            .iter()
            .map(|certificate| CertificateDer::from(certificate.as_der().to_vec()))
            .collect(),
        key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(chain.key().as_der().to_vec())),
    })
}

impl TlsOptions {
    /// What these options say, with every file they name read.
    pub fn load(&self) -> Result<TlsConfig, OptionRefusal> {
        let accept_any_server = self.allow_unsafe_connection.unwrap_or(false);
        if accept_any_server && self.ca_cert_path.is_some() {
            return Err(OptionRefusal::new(
                "CaCertPath",
                "it names roots to verify against, and AllowUnsafeConnection verifies nothing",
            ));
        }

        if self.ca_store.is_some() && (self.ca_cert_path.is_some() || accept_any_server) {
            return Err(OptionRefusal::new(
                "CaStore",
                "it names a root, and so does CaCertPath, or AllowUnsafeConnection verifies nothing",
            ));
        }
        if self.cert_store.is_some()
            && (self.cert_pem.is_some() || self.key_pem.is_some() || self.cert_p12.is_some())
        {
            return Err(OptionRefusal::new(
                "CertStore",
                "it names a client identity, and so do CertPem, KeyPem or CertP12",
            ));
        }

        let roots = match (&self.ca_cert_path, &self.ca_store) {
            (Some(path), _) => certificates("CaCertPath", path)?,
            (None, Some(store)) => {
                vec![store.root().map_err(|refused| refused.under("CaStore"))?]
            }
            (None, None) => Vec::new(),
        };

        if self.cert_p12.is_some() && (self.cert_pem.is_some() || self.key_pem.is_some()) {
            return Err(OptionRefusal::new(
                "CertP12",
                "it names a client identity, and so do CertPem and KeyPem",
            ));
        }
        if self.cert_p12.is_none() && self.cert_p12_password.is_some() {
            return Err(OptionRefusal::new(
                "CertP12Password",
                "it is set, and CertP12 names no bundle",
            ));
        }

        let identity = match (&self.cert_pem, &self.key_pem) {
            (None, None) => match (&self.cert_p12, &self.cert_store) {
                (Some(path), _) => Some(pkcs12("CertP12", path, self.cert_p12_password.as_ref())?),
                (None, Some(store)) => Some(
                    store
                        .identity()
                        .map_err(|refused| refused.under("CertStore"))?,
                ),
                (None, None) => None,
            },
            (Some(_), None) => {
                return Err(OptionRefusal::new(
                    "KeyPem",
                    "it is missing, and CertPem names a certificate that needs its key",
                ))
            }
            (None, Some(_)) => {
                return Err(OptionRefusal::new(
                    "CertPem",
                    "it is missing, and KeyPem names a key that needs its certificate",
                ))
            }
            (Some(cert), Some(key)) => {
                let chain = certificates("CertPem", cert)?;
                let key =
                    PrivateKeyDer::from_pem_slice(&read("KeyPem", key)?).map_err(|error| {
                        OptionRefusal::new(
                            "KeyPem",
                            format!("the file it names holds no key PEM can carry: {error}"),
                        )
                    })?;
                Some(ClientIdentity { chain, key })
            }
        };

        if let Some(name) = &self.override_target_name {
            crate::http2::verified_name(name)
                .map_err(|refused| OptionRefusal::new("OverrideTargetName", refused.to_string()))?;
        }

        Ok(TlsConfig {
            roots,
            accept_any_server,
            identity,
            server_name: self.override_target_name.clone(),
        })
    }
}

impl TcpKeepaliveOptions {
    pub fn to_config(&self) -> Result<TcpConfig, OptionRefusal> {
        let keepalive = duration("IdleSeconds", self.idle_seconds, 1.0, Some(32767.0))?;
        let keepalive_interval =
            duration("IntervalSeconds", self.interval_seconds, 1.0, Some(32767.0))?;
        let keepalive_retries = match self.retries {
            None => None,
            Some(retries) if !(1..=127).contains(&retries) => {
                return Err(OptionRefusal::new(
                    "Retries",
                    format!("{retries} has to be from 1 to 127"),
                ))
            }
            Some(retries) => Some(retries as u32),
        };
        if keepalive.is_none() {
            for (key, set) in [
                ("IntervalSeconds", keepalive_interval.is_some()),
                ("Retries", keepalive_retries.is_some()),
            ] {
                if set {
                    return Err(OptionRefusal::new(
                        key,
                        "it needs IdleSeconds, without which the probes start at the operating \
                         system's idle time",
                    ));
                }
            }
        }
        Ok(TcpConfig {
            keepalive,
            keepalive_interval,
            keepalive_retries,
        })
    }
}

impl Http2Options {
    pub fn to_config(&self) -> Result<Http2Config, OptionRefusal> {
        let defaults = Http2Config::default();
        let window = |key: &str, asked: Option<i32>, least: i32, default: u32| match asked {
            None => Ok(default),
            Some(size) if size < least => Err(OptionRefusal::new(
                key,
                format!("{size} has to be at least {least}"),
            )),
            Some(size) => Ok(size as u32),
        };
        Ok(Http2Config {
            keep_alive_interval: duration(
                "KeepAliveIntervalSeconds",
                self.keep_alive_interval_seconds,
                1e-9,
                None,
            )?,
            keep_alive_timeout: duration(
                "KeepAliveTimeoutSeconds",
                self.keep_alive_timeout_seconds,
                1e-9,
                None,
            )?
            .unwrap_or(defaults.keep_alive_timeout),
            keep_alive_while_idle: self
                .keep_alive_while_idle
                .unwrap_or(defaults.keep_alive_while_idle),
            stream_window: window(
                "StreamWindowSize",
                self.stream_window_size,
                1,
                defaults.stream_window,
            )?,
            connection_window: window(
                "ConnectionWindowSize",
                self.connection_window_size,
                65_535,
                defaults.connection_window,
            )?,
            idle_timeout: duration("IdleTimeoutSeconds", self.idle_timeout_seconds, 1e-9, None)?,
            write_coalescing: match self.write_coalescing_bytes {
                None => defaults.write_coalescing,
                Some(bytes) if bytes < 0 => {
                    return Err(OptionRefusal::new(
                        "WriteCoalescingBytes",
                        format!("{bytes} has to be at least 0"),
                    ))
                }
                Some(bytes) => bytes as usize,
            },
        })
    }
}

/// What a caller may set on one channel.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(rename_all = "PascalCase", deny_unknown_fields)
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct ChannelOptions {
    /// What the transport does, beyond reaching the endpoint.
    ///
    /// Defaults to `{}`, which leaves each of its options at its own default.
    #[cfg_attr(feature = "serde", serde(default))]
    pub transport: TransportOptions,

    /// What this client calls itself in `user-agent`.
    ///
    /// Defaults to `armonik-transport/` followed by the engine's version.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "String", length(min = 1)))]
    pub user_agent: Option<String>,

    /// The largest message this client will accept, in bytes.
    ///
    /// Defaults to 4194304, 4 MiB. No upper bound, because the largest a caller can name is a
    /// channel that refuses nothing. Zero is refused: it is a channel that can receive no message
    /// at all.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "i32", range(min = 1)))]
    pub max_receive_message_size: Option<i32>,

    /// The deadline of a call that states none, counted from its start: the call ends
    /// `DEADLINE_EXCEEDED` once it passes, and the server is told what was left of it when the
    /// call started as `grpc-timeout`. A call's own deadline takes its place, and a call that
    /// states none takes this one.
    ///
    /// Defaults to none, a call waiting as long as its answer takes; at least a nanosecond, the
    /// finest duration the engine holds.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Seconds", extend("minimum" = 1e-9))
    )]
    pub default_deadline_seconds: Option<Seconds>,

    /// How many messages a call may have sent and unacquitted at once.
    ///
    /// Defaults to 1.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "i32", range(min = 1, max = LARGEST_WINDOW))
    )]
    pub max_sends_in_flight: Option<i32>,

    /// How many of a call's payloads the host may hold at once, delivered and not yet given back.
    /// The terminal status takes none, so a host holds at most one more.
    ///
    /// Defaults to 4.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "i32", range(min = 1, max = LARGEST_WINDOW))
    )]
    pub delivery_credits: Option<i32>,

    /// The HTTP/2 session the channel's calls share.
    ///
    /// Defaults to `{}`, which leaves each of its options at its own default.
    #[cfg_attr(feature = "serde", serde(default))]
    pub http2: Http2Options,

    /// When a failed call is sent again.
    ///
    /// Defaults to `{}`: five attempts in all, as `GrpcClient` has them. A call its peer never
    /// processed goes again besides, whatever `MaxAttempts` is, while every message it sent is
    /// kept.
    #[cfg_attr(feature = "serde", serde(default))]
    pub retry: RetryOptions,

    /// Whether the channel starts dialling its endpoint as it is created rather than at its first
    /// call, which then finds the session open or joins the dial under way. A dial that fails is
    /// not reported: the first call dials again and reports what it meets.
    ///
    /// Defaults to false.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "bool"))]
    pub connect_eagerly: Option<bool>,
}

/// The schema of [`ChannelOptions`], as the committed file holds it.
///
/// Rendered here rather than by whoever asks, so the file, the test that checks it and any
/// other reader are looking at the same bytes.
///
/// A default is stated in its option's description and nowhere else in the schema: applying it
/// is the reader's, and a `default` keyword is one a validator or a generator could act on.
#[cfg(feature = "schema")]
pub fn schema() -> String {
    let schema = schemars::generate::SchemaSettings::default()
        .with_transform(schemars::transform::RecursiveTransform(
            |schema: &mut schemars::Schema| {
                schema.remove("default");
            },
        ))
        .into_generator()
        .into_root_schema_for::<ChannelOptions>();
    let mut rendered = serde_json::to_string_pretty(&schema).expect("a schema renders");
    rendered.push('\n');
    rendered
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of its own per test, so tests running at once write no file another reads.
    fn scratch(test: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "armonik-transport-options-{}-{test}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("a scratch directory");
        directory
    }

    fn write(directory: &std::path::Path, name: &str, content: &str) -> String {
        let path = directory.join(name);
        std::fs::write(&path, content).expect("a scratch file");
        path.to_string_lossy().into_owned()
    }

    /// A certificate and its key, as PEM.
    fn pem_pair() -> (String, String) {
        let key = rcgen::KeyPair::generate().expect("a key");
        let certificate = rcgen::CertificateParams::new(vec!["client.test".to_owned()])
            .expect("parameters")
            .self_signed(&key)
            .expect("a certificate");
        (certificate.pem(), key.serialize_pem())
    }

    /// Removes its directory when it goes.
    struct Scratch(std::path::PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_files_the_tls_options_name_are_read_into_the_engine_configuration() {
        let directory = scratch("read");
        let _gone = Scratch(directory.clone());
        let (certificate, key) = pem_pair();
        let two = format!("{certificate}{certificate}");
        let options = TlsOptions {
            ca_cert_path: Some(write(&directory, "ca.pem", &certificate)),
            cert_pem: Some(write(&directory, "chain.pem", &two)),
            key_pem: Some(write(&directory, "key.pem", &key)),
            override_target_name: Some("server.test".to_owned()),
            ..TlsOptions::default()
        };

        let config = options.load().expect("readable files");
        assert_eq!(config.roots.len(), 1);
        let identity = config.identity.expect("an identity");
        assert_eq!(
            identity.chain.len(),
            2,
            "the whole chain, in the file's order"
        );
        assert_eq!(config.server_name.as_deref(), Some("server.test"));
        assert!(!config.accept_any_server);
    }

    #[test]
    fn a_tls_refusal_names_its_option_and_never_the_path() {
        let directory = scratch("refused");
        let _gone = Scratch(directory.clone());
        let (certificate, _) = pem_pair();
        let missing = directory
            .join("s3cret-name.pem")
            .to_string_lossy()
            .into_owned();
        let empty = write(&directory, "empty.pem", "no PEM here");
        let certificate = write(&directory, "cert.pem", &certificate);

        for (options, key) in [
            (
                TlsOptions {
                    ca_cert_path: Some(missing.clone()),
                    ..TlsOptions::default()
                },
                "CaCertPath",
            ),
            (
                TlsOptions {
                    ca_cert_path: Some(empty.clone()),
                    ..TlsOptions::default()
                },
                "CaCertPath",
            ),
            (
                TlsOptions {
                    cert_pem: Some(certificate.clone()),
                    ..TlsOptions::default()
                },
                "KeyPem",
            ),
            (
                TlsOptions {
                    key_pem: Some(missing.clone()),
                    ..TlsOptions::default()
                },
                "CertPem",
            ),
            (
                TlsOptions {
                    cert_pem: Some(certificate.clone()),
                    key_pem: Some(certificate.clone()),
                    ..TlsOptions::default()
                },
                "KeyPem",
            ),
            (
                TlsOptions {
                    cert_pem: Some(certificate.clone()),
                    key_pem: Some(missing.clone()),
                    ..TlsOptions::default()
                },
                "KeyPem",
            ),
            (
                TlsOptions {
                    ca_cert_path: Some(certificate.clone()),
                    allow_unsafe_connection: Some(true),
                    ..TlsOptions::default()
                },
                "CaCertPath",
            ),
            (
                TlsOptions {
                    override_target_name: Some("-nope-".to_owned()),
                    ..TlsOptions::default()
                },
                "OverrideTargetName",
            ),
        ] {
            let refused = options.load().expect_err(key);
            assert_eq!(refused.key(), key, "{refused}");
            let said = refused.to_string();
            assert!(
                !said.contains("  "),
                "a refusal reads as one sentence: {said}"
            );
            for path in [&missing, &empty, &certificate] {
                assert!(!said.contains(path.as_str()), "{said}");
            }
            assert!(!said.contains("s3cret"), "{said}");
        }
    }

    /// A PKCS#12 bundle of `key` and `certificates`, protected by `password`.
    fn p12_bundle(
        key: &rcgen::KeyPair,
        certificates: &[&rcgen::Certificate],
        password: &str,
    ) -> Vec<u8> {
        let chain = p12_keystore::PrivateKeyChain::new(
            [1u8].as_slice(),
            p12_keystore::PrivateKey::from_der(&key.serialize_der()).expect("a PKCS#8 key"),
            certificates.iter().map(|certificate| {
                p12_keystore::Certificate::from_der(certificate.der().as_ref())
                    .expect("an X.509 certificate")
            }),
        );
        let mut store = p12_keystore::KeyStore::new();
        store.add_entry(
            "identity",
            p12_keystore::KeyStoreEntry::PrivateKeyChain(chain),
        );
        store.writer(password).write().expect("a bundle")
    }

    fn bundled(directory: &std::path::Path, name: &str, bundle: Vec<u8>) -> String {
        let path = directory.join(name);
        std::fs::write(&path, bundle).expect("a scratch file");
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn a_pkcs12_bundle_is_read_into_the_identity_it_carries() {
        let directory = scratch("p12");
        let _gone = Scratch(directory.clone());
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(["client.test".to_owned()]).expect("an identity");

        let options = TlsOptions {
            cert_p12: Some(bundled(
                &directory,
                "identity.p12",
                p12_bundle(&signing_key, &[&cert], "s3cret-word"),
            )),
            cert_p12_password: Some(Password::new("s3cret-word")),
            ..TlsOptions::default()
        };
        let identity = options
            .load()
            .expect("a bundle")
            .identity
            .expect("an identity");
        assert_eq!(identity.chain.len(), 1);
        assert_eq!(identity.chain[0].as_ref(), cert.der().as_ref());
        let PrivateKeyDer::Pkcs8(key) = &identity.key else {
            panic!("the bundle carried a PKCS#8 key");
        };
        assert_eq!(key.secret_pkcs8_der(), signing_key.serialize_der());

        let unprotected = TlsOptions {
            cert_p12: Some(bundled(
                &directory,
                "open.p12",
                p12_bundle(&signing_key, &[&cert], ""),
            )),
            ..TlsOptions::default()
        };
        assert!(
            unprotected.load().expect("no password").identity.is_some(),
            "no password opens a bundle written with the empty one"
        );
    }

    #[test]
    fn a_pkcs12_refusal_names_its_option_and_quotes_neither_password_nor_path() {
        let directory = scratch("p12-refused");
        let _gone = Scratch(directory.clone());
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(["client.test".to_owned()]).expect("an identity");
        let protected = bundled(
            &directory,
            "s3cret-name.p12",
            p12_bundle(&signing_key, &[&cert], "s3cret-word"),
        );
        let empty = bundled(
            &directory,
            "empty.p12",
            p12_keystore::KeyStore::new()
                .writer("s3cret-word")
                .write()
                .expect("a bundle"),
        );
        let garbage = bundled(&directory, "garbage.p12", b"not a bundle".to_vec());
        let two = {
            let entry = |id: u8| {
                p12_keystore::KeyStoreEntry::PrivateKeyChain(p12_keystore::PrivateKeyChain::new(
                    [id].as_slice(),
                    p12_keystore::PrivateKey::from_der(&signing_key.serialize_der())
                        .expect("a PKCS#8 key"),
                    [p12_keystore::Certificate::from_der(cert.der().as_ref())
                        .expect("an X.509 certificate")],
                ))
            };
            let mut store = p12_keystore::KeyStore::new();
            store.add_entry("first", entry(1));
            store.add_entry("second", entry(2));
            bundled(
                &directory,
                "two.p12",
                store.writer("s3cret-word").write().expect("a bundle"),
            )
        };

        for (options, key) in [
            (
                TlsOptions {
                    cert_p12: Some(protected.clone()),
                    cert_p12_password: Some(Password::new("hunter2")),
                    ..TlsOptions::default()
                },
                "CertP12",
            ),
            (
                TlsOptions {
                    cert_p12: Some(empty.clone()),
                    cert_p12_password: Some(Password::new("s3cret-word")),
                    ..TlsOptions::default()
                },
                "CertP12",
            ),
            (
                TlsOptions {
                    cert_p12: Some(garbage.clone()),
                    ..TlsOptions::default()
                },
                "CertP12",
            ),
            (
                TlsOptions {
                    cert_p12: Some(two.clone()),
                    cert_p12_password: Some(Password::new("s3cret-word")),
                    ..TlsOptions::default()
                },
                "CertP12",
            ),
            (
                TlsOptions {
                    cert_p12: Some(protected.clone()),
                    cert_pem: Some(protected.clone()),
                    ..TlsOptions::default()
                },
                "CertP12",
            ),
            (
                TlsOptions {
                    cert_p12_password: Some(Password::new("s3cret-word")),
                    ..TlsOptions::default()
                },
                "CertP12Password",
            ),
        ] {
            let refused = options.load().expect_err(key);
            assert_eq!(refused.key(), key, "{refused}");
            let said = refused.to_string();
            for secret in ["s3cret", "hunter2", &protected, &empty, &garbage, &two] {
                assert!(!said.contains(secret), "{said}");
            }
            assert!(
                !format!("{options:?}").contains("s3cret-word"),
                "{options:?}"
            );
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn a_windows_store_is_refused_off_windows_by_the_option_naming_it() {
        let mut store = StoreCertificate::default();
        store.friendly_name = Some("anything".to_owned());
        for (options, unit) in [
            (
                TlsOptions {
                    cert_store: Some(store.clone()),
                    ..TlsOptions::default()
                },
                "CertStore",
            ),
            (
                TlsOptions {
                    ca_store: Some(store.clone()),
                    ..TlsOptions::default()
                },
                "CaStore",
            ),
        ] {
            let refused = options.load().expect_err(unit);
            assert!(refused.key().starts_with(unit), "{refused}");
            assert!(refused.to_string().contains("Windows only"), "{refused}");
        }
    }

    fn proxy(
        address: Option<&str>,
        username: Option<&str>,
        password: Option<&str>,
    ) -> ProxyOptions {
        ProxyOptions {
            address: address.map(str::to_owned),
            username: username.map(str::to_owned),
            password: password.map(Password::new),
        }
    }

    #[test]
    fn a_proxy_url_becomes_the_proxy_tunnelled_through_with_its_credentials() {
        let explicit = |config: ProxyConfig| match config.source {
            ProxySource::Explicit(uri) => (uri.to_string(), config.username, config.password),
            other => panic!("{other:?}"),
        };

        let (uri, username, password) = explicit(
            proxy(Some("proxy.test:3128"), Some("alice"), Some("s3cret"))
                .to_config()
                .expect("a proxy"),
        );
        assert_eq!(uri, "http://proxy.test:3128/", "http:// is assumed");
        assert_eq!(
            (username.as_str(), password.expose_secret()),
            ("alice", "s3cret")
        );

        let (uri, username, password) = explicit(
            proxy(Some("http://alice:s%40cret@proxy.test:3128"), None, None)
                .to_config()
                .expect("a proxy"),
        );
        assert_eq!(
            uri, "http://proxy.test:3128/",
            "the URL keeps no credential"
        );
        assert_eq!(
            (username.as_str(), password.expose_secret()),
            ("alice", "s@cret")
        );

        for none in ["none", "NONE"] {
            let config = proxy(Some(none), None, None).to_config().expect("no proxy");
            assert_eq!(config.source, ProxySource::Disabled, "{none:?}");
        }
        let config = proxy(Some("none"), Some("alice"), Some("s3cret"))
            .to_config()
            .expect("credentials beside `none` are left unread");
        assert_eq!(config.source, ProxySource::Disabled);

        let (uri, _, _) = explicit(
            proxy(Some("http://[::1]:3128"), None, None)
                .to_config()
                .expect("a bracketed IPv6 proxy"),
        );
        assert_eq!(uri, "http://[::1]:3128/");

        let refused = proxy(Some("proxy.test:3128"), Some("corp:alice"), Some("s3cret"))
            .to_config()
            .expect_err("a `:` in the username");
        assert_eq!(refused.key(), "Username");
    }

    #[test]
    fn no_address_or_system_is_the_environments_proxy_with_the_dedicated_credentials() {
        for address in [None, Some("system"), Some("System")] {
            let config = proxy(address, Some("alice"), None)
                .to_config()
                .expect("the environment's proxy");
            assert_eq!(config.source, ProxySource::System, "{address:?}");
            assert_eq!(config.username, "alice");
            assert_eq!(config.password.expose_secret(), "");
        }
        let refused = proxy(None, Some("corp:alice"), None)
            .to_config()
            .expect_err("a `:` in the username");
        assert_eq!(refused.key(), "Username");
    }

    #[test]
    fn a_debug_print_shows_the_proxy_and_never_its_password() {
        for (address, shown) in [
            (
                "http://alice:s3cret@proxy.test:3128",
                "http://***@proxy.test:3128",
            ),
            ("http://proxy.test:s3cret", "http://proxy.test:***"),
            ("http://[::1]:3128", "http://[::1]:3128"),
            ("http://[::1]:s3cret", "http://[::1]:***"),
            ("proxy.test:3128", "proxy.test:3128"),
        ] {
            let printed = format!("{:?}", proxy(Some(address), None, None));
            assert!(printed.contains(shown), "{address}: {printed}");
            assert!(!printed.contains("s3cret"), "{printed}");
        }
    }

    #[test]
    fn a_proxy_refusal_names_the_address_and_quotes_neither_it_nor_a_password() {
        for options in [
            proxy(Some("https://proxy.test:443"), None, None),
            proxy(Some("http://alice:s3cret@proxy.test"), Some("bob"), None),
            proxy(Some("http://alice:s3cret@proxy.test"), None, Some("other")),
            proxy(Some("http://proxy.test:99999"), None, None),
            proxy(Some("http://proxy.test:s3cret"), None, None),
            proxy(Some("http://:3128"), None, None),
            proxy(Some("not a url"), None, None),
            proxy(Some("http://proxy.test:3128/pac.js"), None, None),
            proxy(Some("http://proxy.test:3128/?s3cret"), None, None),
            proxy(Some("http://proxy.test:3128#s3cret"), None, None),
            proxy(Some("http://alice:%FF@proxy.test:3128"), None, None),
            proxy(
                Some("http://corp%3Aalice:s3cret@proxy.test:3128"),
                None,
                None,
            ),
        ] {
            let refused = options.to_config().expect_err("refused");
            assert_eq!(refused.key(), "Address", "{options:?}: {refused}");
            let said = refused.to_string();
            assert!(!said.contains("s3cret"), "{said}");
            assert!(!said.contains("proxy.test"), "{said}");
            assert!(!format!("{options:?}").contains("s3cret"), "{options:?}");
        }
    }

    #[test]
    fn a_unit_refusal_is_named_from_where_the_unit_is_embedded() {
        let refused = TcpKeepaliveOptions {
            interval_seconds: Some(Seconds(5.0)),
            ..TcpKeepaliveOptions::default()
        }
        .to_config()
        .expect_err("an interval with no keepalive")
        .under("Transport.TcpKeepalive");
        assert_eq!(refused.key(), "Transport.TcpKeepalive.IntervalSeconds");
        assert!(!refused.to_string().contains("  "), "{refused}");
        assert!(refused
            .to_string()
            .starts_with("Transport.TcpKeepalive.IntervalSeconds"));
    }

    #[test]
    fn the_keepalive_options_become_the_socket_configuration() {
        let config = TcpKeepaliveOptions {
            idle_seconds: Some(Seconds(30.0)),
            interval_seconds: Some(Seconds(5.0)),
            retries: Some(3),
        }
        .to_config()
        .expect("admissible");
        assert_eq!(config.keepalive, Some(Duration::from_secs(30)));
        assert_eq!(config.keepalive_interval, Some(Duration::from_secs(5)));
        assert_eq!(config.keepalive_retries, Some(3));

        for refused in [
            TcpKeepaliveOptions {
                idle_seconds: Some(Seconds(0.5)),
                ..TcpKeepaliveOptions::default()
            },
            TcpKeepaliveOptions {
                retries: Some(3),
                ..TcpKeepaliveOptions::default()
            },
            TcpKeepaliveOptions {
                idle_seconds: Some(Seconds(30.0)),
                retries: Some(0),
                ..TcpKeepaliveOptions::default()
            },
            TcpKeepaliveOptions {
                idle_seconds: Some(Seconds(32768.0)),
                ..TcpKeepaliveOptions::default()
            },
            TcpKeepaliveOptions {
                idle_seconds: Some(Seconds(30.0)),
                retries: Some(128),
                ..TcpKeepaliveOptions::default()
            },
        ] {
            assert!(refused.to_config().is_err(), "{refused:?}");
        }
    }

    #[test]
    fn the_retry_options_become_the_policy_and_one_that_cannot_back_off_is_refused() {
        let config = RetryOptions {
            max_attempts: Some(3),
            initial_backoff_seconds: Some(Seconds(0.5)),
            max_backoff_seconds: Some(Seconds(2.0)),
            backoff_multiplier: Some(2.0),
            call_replay_bytes: Some(10),
            channel_replay_bytes: Some(100),
        }
        .to_config()
        .expect("admissible");
        assert_eq!(config.max_attempts, 3);
        assert_eq!(config.initial_backoff, Duration::from_millis(500));
        assert_eq!(config.max_backoff, Duration::from_secs(2));
        assert_eq!(config.backoff_multiplier, 2.0);
        assert_eq!(
            (config.call_replay_bytes, config.channel_replay_bytes),
            (10, 100)
        );
        assert_eq!(
            RetryOptions::default().to_config().expect("the defaults"),
            RetryConfig::default()
        );

        for (options, key) in [
            (
                RetryOptions {
                    max_attempts: Some(0),
                    ..RetryOptions::default()
                },
                "MaxAttempts",
            ),
            (
                RetryOptions {
                    initial_backoff_seconds: Some(Seconds(10.0)),
                    ..RetryOptions::default()
                },
                "MaxBackoffSeconds",
            ),
            (
                RetryOptions {
                    backoff_multiplier: Some(0.5),
                    ..RetryOptions::default()
                },
                "BackoffMultiplier",
            ),
            (
                RetryOptions {
                    backoff_multiplier: Some(f64::INFINITY),
                    ..RetryOptions::default()
                },
                "BackoffMultiplier",
            ),
            (
                RetryOptions {
                    call_replay_bytes: Some(-1),
                    ..RetryOptions::default()
                },
                "CallReplayBytes",
            ),
        ] {
            let refused = options.to_config().expect_err(key);
            assert_eq!(refused.key(), key, "{refused}");
        }
    }

    #[test]
    fn the_http2_options_become_the_session_configuration() {
        let config = Http2Options {
            keep_alive_interval_seconds: Some(Seconds(10.0)),
            keep_alive_timeout_seconds: Some(Seconds(2.5)),
            keep_alive_while_idle: Some(true),
            stream_window_size: Some(1024),
            connection_window_size: Some(65_535),
            idle_timeout_seconds: Some(Seconds(300.0)),
            write_coalescing_bytes: Some(0),
        }
        .to_config()
        .expect("admissible");
        assert_eq!(config.idle_timeout, Some(Duration::from_secs(300)));
        assert_eq!(config.write_coalescing, 0);
        assert_eq!(config.keep_alive_interval, Some(Duration::from_secs(10)));
        assert_eq!(config.keep_alive_timeout, Duration::from_millis(2500));
        assert!(config.keep_alive_while_idle);
        assert_eq!(
            (config.stream_window, config.connection_window),
            (1024, 65_535)
        );

        assert_eq!(
            Http2Options::default().to_config().expect("the defaults"),
            Http2Config::default()
        );

        let refused = Http2Options {
            connection_window_size: Some(65_534),
            ..Http2Options::default()
        }
        .to_config()
        .expect_err("below the window every connection starts with");
        assert_eq!(refused.key(), "ConnectionWindowSize");

        let refused = Http2Options {
            write_coalescing_bytes: Some(-1),
            ..Http2Options::default()
        }
        .to_config()
        .expect_err("a negative size");
        assert_eq!(refused.key(), "WriteCoalescingBytes");
    }

    /// The committed schema is what generates the C# class, so it has to be what these types
    /// say - and it cannot be regenerated at build time, since the generator that reads it runs
    /// before any build on a fresh clone.
    #[cfg(feature = "schema")]
    #[test]
    fn the_committed_schema_is_the_one_the_types_describe() {
        let committed = include_str!("../options.schema.json").replace("\r\n", "\n");

        assert_eq!(
            committed,
            schema(),
            "the options changed and the schema did not; write it again with\n  \
             cargo run -p armonik-transport --features schema --example schema -- \
             packages/rust/armonik-transport/options.schema.json"
        );
    }

    /// Every option the schema declares is one `serde` reads, under the name the schema spells.
    ///
    /// The two derives are separate readings of the same fields, and this crate makes them differ
    /// on purpose - `schemars(with = "i32")` states a schema the field's own type would not. A
    /// name they stopped agreeing on would be an option the generated C# sets, the schema admits,
    /// and `deny_unknown_fields` refuses at the far end of the ABI.
    #[cfg(all(feature = "schema", feature = "serde"))]
    #[test]
    fn every_option_the_schema_declares_is_one_serde_reads() {
        let schema: serde_json::Value =
            serde_json::from_str(&schema()).expect("the schema is a document");

        let document = a_value_for(&schema, &schema);

        let read = serde_json::from_value::<ChannelOptions>(document.clone());

        assert!(
            read.is_ok(),
            "the schema declares {document}, which serde refuses: {}",
            read.unwrap_err()
        );
    }

    /// A value each property of `node` admits, as one document naming all of them.
    ///
    /// Values rather than a name list, because `deny_unknown_fields` refuses a name and the type
    /// refuses a value, and only a document carrying both exercises the two.
    #[cfg(all(feature = "schema", feature = "serde"))]
    fn a_value_for(node: &serde_json::Value, root: &serde_json::Value) -> serde_json::Value {
        use serde_json::{json, Value};

        // A `$ref` states the type and the property beside it states its bounds, so the reference
        // is followed only for what the property does not say.
        let node = match node.get("$ref").and_then(Value::as_str) {
            Some(reference) => {
                let name = reference
                    .rsplit('/')
                    .next()
                    .expect("a reference names something");
                &root["$defs"][name]
            }
            None => node,
        };

        match node.get("type").and_then(Value::as_str) {
            Some("object") | None => {
                let properties = node
                    .get("properties")
                    .and_then(Value::as_object)
                    .expect("an object states its properties");

                Value::Object(
                    properties
                        .iter()
                        .map(|(name, property)| (name.clone(), a_value_for(property, root)))
                        .collect(),
                )
            }
            // A value every bound in this schema admits: an integer's `minimum` is 1 where it is
            // stated, and the timeout's is a nanosecond.
            Some("integer") => json!(1),
            Some("number") => json!(1.0),
            Some("string") => json!("x"),
            Some("boolean") => json!(true),
            Some(other) => panic!("`{other}` is a type this test states no value for"),
        }
    }

    /// The bound the schema states has to be one every target can honour, and the tighter of the
    /// two targets is what it states - so on a 64-bit host this passes with room to spare and on
    /// a 32-bit one it passes by exactly one.
    #[test]
    fn a_window_the_schema_admits_is_one_a_semaphore_admits() {
        assert!(
            (LARGEST_WINDOW as usize) < tokio::sync::Semaphore::MAX_PERMITS,
            "the schema admits {LARGEST_WINDOW}, which a semaphore of {} would refuse",
            tokio::sync::Semaphore::MAX_PERMITS
        );
    }
}
