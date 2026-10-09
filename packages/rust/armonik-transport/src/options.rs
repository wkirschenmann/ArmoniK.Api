//! The options a caller sets on a channel, as a document carries them.
//!
//! Structured and typed, because the schema derived from these types is what generates the
//! options class a .NET caller fills in: a number is a number, a group of options is an object,
//! and every constraint that can be said here is said here rather than only in the code that
//! enforces it. What a type cannot say - that an endpoint names a scheme this engine speaks -
//! the transport says, by option name.
//!
//! The `configuration` loader refuses a key no type declares, at the root of a document as below
//! it, naming its path, and so the schema states `additionalProperties: false` for every object. An
//! alternative - how the server is verified, who the client is, which proxy - refuses a key that
//! names none of its variants.

use std::time::Duration;

use hyper::Uri;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use secrecy::ExposeSecret;

use crate::grpc::{AdaptiveConfig, Cause, GrpcStatusCode, ReplayConfig, RetryConfig};
use crate::http2::{
    ClientIdentity, FixedWindows, Http2Config, ProxyConfig, ProxySource, ReceiveWindows, TcpConfig,
    TlsConfig, LARGEST_FRAMES_PER_WRITE,
};

/// The largest window either side of a call may be given.
///
/// A window becomes a `tokio` semaphore, which refuses more than `Semaphore::MAX_PERMITS`
/// permits, and that limit is `usize::MAX >> 3` - so it is far larger on a 64-bit target than on
/// a 32-bit one. The schema is one file for every target, so the bound it states is the tighter
/// of the two; `a_window_the_schema_admits_is_one_a_semaphore_admits` is what keeps that true.
pub const LARGEST_WINDOW: i32 = 536_870_910;

/// The most KiB a stream's send buffer may hold: the session counts it in 32 bits.
pub const LARGEST_STREAM_BUFFER_KIB: i32 = 4_194_303;

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
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[serde(transparent)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(extend("exclusiveMaximum" = 18446744073709551616.0))
)]
pub struct Seconds(pub f64);

/// Refused at 2^64 and above, the ceiling the type states, and when it is not a number, which
/// fails the comparison.
impl<'de> serde::Deserialize<'de> for Seconds {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let seconds = f64::deserialize(deserializer)?;
        if seconds < 18_446_744_073_709_551_616.0 {
            Ok(Self(seconds))
        } else {
            Err(serde::de::Error::custom(
                "it has to be less than 18446744073709551616 seconds",
            ))
        }
    }
}

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
/// The endpoint is not here: a channel is opened on its own, or on the Endpoint of its runtime's
/// options. Every option has a default, so naming none of them is a valid configuration.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct TransportOptions {
    /// How long a dial may take before it is given up on.
    ///
    /// Defaults to 60, and at least a nanosecond, the finest duration the engine holds: a shorter
    /// one could round to zero, which no dial could beat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    // Set beside the `$ref` that `with` writes, where `range` does not reach.
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Seconds", extend("minimum" = 1e-9))
    )]
    #[serde(deserialize_with = "within::nanosecond")]
    pub connect_timeout_seconds: Option<Seconds>,

    /// How an `https://` endpoint is secured.
    ///
    /// Defaults to `{}`: the server verified against the system's roots under the endpoint's
    /// host, and no client certificate. Refused for an `http://` endpoint unless it sets nothing.
    #[serde(default)]
    pub tls: TlsOptions,

    /// The socket's keepalive.
    ///
    /// Defaults to `"None"`: no probe is sent.
    #[serde(
        default,
        deserialize_with = "alternative::optional",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "TcpKeepalive"))]
    pub tcp_keepalive: Option<TcpKeepalive>,

    /// The HTTP proxy every dial tunnels through.
    ///
    /// Defaults to `{"System": {}}`: the proxy the system names, if any, with no credentials of
    /// its own.
    #[serde(
        default,
        deserialize_with = "alternative::optional",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "ProxyOptions"))]
    pub proxy: Option<ProxyOptions>,

    /// Whether the channel starts dialling its endpoint as it is created rather than at its first
    /// call, which then finds the session open or joins the dial under way. A dial that fails is
    /// not reported: the first call dials again and reports what it meets.
    ///
    /// Defaults to false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "bool"))]
    pub connect_eagerly: Option<bool>,
}

/// An HTTP proxy, which a dial tunnels through with `CONNECT`, so TLS stays end to end with the
/// server.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum ProxyOptions {
    /// No proxy: every dial goes to the endpoint itself.
    None,

    /// The proxy the system names for the endpoint, if any.
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
    System(ProxyCredentials),

    /// The proxy at an address that carries no credentials, with its own beside it, if any.
    Url(ProxyUrl),

    /// The proxy at an address that carries its credentials as `user:password@`, percent-encoded,
    /// which a serialized document then carries too.
    UrlWithCredentials(CredentialedUrl),
}

impl Default for ProxyOptions {
    fn default() -> Self {
        Self::System(ProxyCredentials::default())
    }
}

/// The credentials the system's proxy is authenticated to with, by `Basic`.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct ProxyCredentials {
    /// The username, which `Basic` forbids a `:` in.
    ///
    /// Ignored when the system names no proxy. Beside the environment's proxy, it takes the place
    /// of the username that proxy's URL carries; beside the one Windows' settings name, it is the
    /// username.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub username: Option<String>,

    /// The password that goes with `Username`.
    ///
    /// Ignored when the system names no proxy. Beside the environment's proxy, it takes the place
    /// of the password that proxy's URL carries; beside the one Windows' settings name, it is the
    /// password.
    #[serde(default, skip_serializing)]
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub password: Option<Password>,
}

/// A proxy named by its address.
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct ProxyUrl {
    /// The proxy's `http://` URL, with no path and no `user:password@`; `http://` is assumed when
    /// no scheme is written.
    // `writeOnly`, so a generated binding never prints one written with credentials by mistake.
    #[cfg_attr(
        feature = "schema",
        schemars(length(min = 1), extend("writeOnly" = true))
    )]
    pub address: String,

    /// The username the proxy is authenticated to with, by `Basic`, which forbids a `:` in it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub username: Option<String>,

    /// The password that goes with `Username`.
    #[serde(default, skip_serializing)]
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub password: Option<Password>,
}

/// A proxy's `http://` URL that carries its credentials, as `user:password@`, percent-encoded;
/// `http://` is assumed when no scheme is written.
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
// `writeOnly`, so a generated binding treats it as the secret it holds.
#[cfg_attr(
    feature = "schema",
    schemars(transparent, extend("writeOnly" = true))
)]
pub struct CredentialedUrl(#[cfg_attr(feature = "schema", schemars(length(min = 1)))] pub String);

/// The address is printed elided, since it holds a password.
impl std::fmt::Debug for CredentialedUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("CredentialedUrl")
            .field(&elided(&self.0))
            .finish()
    }
}

impl ProxyUrl {
    pub fn new(address: impl Into<String>) -> Self {
        Self {
            address: address.into(),
            username: None,
            password: None,
        }
    }
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
impl std::fmt::Debug for ProxyUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyUrl")
            .field("address", &elided(&self.address))
            .field("username", &self.username)
            .field("password", &self.password)
            .finish()
    }
}

impl ProxyOptions {
    /// The proxy these options name. A refusal never quotes the address, which may hold a
    /// password.
    pub fn to_config(&self) -> Result<ProxyConfig, OptionRefusal> {
        match self {
            Self::None => Ok(ProxyConfig::default()),
            Self::System(credentials) => authenticated(
                ProxySource::System,
                &credentials.username,
                &credentials.password,
            )
            .map_err(|refused| refused.under("System")),
            Self::Url(url) => url.to_config().map_err(|refused| refused.under("Url")),
            Self::UrlWithCredentials(url) => url.to_config(),
        }
    }
}

impl ProxyUrl {
    fn to_config(&self) -> Result<ProxyConfig, OptionRefusal> {
        let (proxy, userinfo) = proxy_url("Address", &self.address)?;
        if userinfo.is_some() {
            return Err(OptionRefusal::new(
                "Address",
                "it carries `user:password@`: a proxy whose URL carries its credentials is \
                 UrlWithCredentials, and Url states them as Username and Password",
            ));
        }
        authenticated(ProxySource::Explicit(proxy), &self.username, &self.password)
    }
}

impl CredentialedUrl {
    fn to_config(&self) -> Result<ProxyConfig, OptionRefusal> {
        const KEY: &str = "UrlWithCredentials";
        let (proxy, userinfo) = proxy_url(KEY, &self.0)?;
        let Some(userinfo) = userinfo else {
            return Err(OptionRefusal::new(
                KEY,
                "it carries no `user:password@`: a proxy whose URL carries no credentials is Url",
            ));
        };
        // Strict: a byte that is not UTF-8 would otherwise become a replacement character, and a
        // password the user did not write.
        let decoded = |text: &str| -> Result<String, OptionRefusal> {
            percent_encoding::percent_decode_str(text)
                .decode_utf8()
                .map(|text| text.into_owned())
                .map_err(|_| not_a_proxy_url(KEY))
        };
        let (username, password) = match userinfo.split_once(':') {
            Some((username, password)) => (decoded(username)?, decoded(password)?),
            None => (decoded(&userinfo)?, String::new()),
        };
        if username.contains(':') {
            return Err(OptionRefusal::new(KEY, NO_COLON));
        }
        Ok(ProxyConfig {
            source: ProxySource::Explicit(proxy),
            username,
            password: password.into(),
        })
    }
}

fn not_a_proxy_url(key: &str) -> OptionRefusal {
    OptionRefusal::new(
        key,
        "it is not a proxy URL such as `http://proxy.example.com:3128`",
    )
}

/// The proxy a URL names, without its credentials, and the `user:password@` it carries, if any,
/// refused by `key`. A refusal never quotes the URL, which may hold a password.
fn proxy_url(key: &str, address: &str) -> Result<(Uri, Option<String>), OptionRefusal> {
    let not_a_url = || not_a_proxy_url(key);
    let written = if address.contains("://") {
        address.to_owned()
    } else {
        format!("http://{address}")
    };
    let uri: Uri = written.parse().map_err(|_| not_a_url())?;
    if uri.scheme_str() != Some("http") {
        return Err(OptionRefusal::new(
            key,
            "it has to be an `http://` URL: the `CONNECT` handshake is written in the clear",
        ));
    }
    let authority = uri.authority().ok_or_else(not_a_url)?.as_str();
    // The last `@`, because a password may hold one.
    let (userinfo, host) = match authority.rsplit_once('@') {
        Some((userinfo, host)) => (Some(userinfo.to_owned()), host),
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
            key,
            "it carries a path, a query or a fragment, which a proxy is not addressed by",
        ));
    }
    let proxy = Uri::builder()
        .scheme("http")
        .authority(host)
        .path_and_query("/")
        .build()
        .map_err(|_| not_a_url())?;
    Ok((proxy, userinfo))
}

/// `source`, authenticated to with `username` and `password`, empty when unset.
fn authenticated(
    source: ProxySource,
    username: &Option<String>,
    password: &Option<Password>,
) -> Result<ProxyConfig, OptionRefusal> {
    let username = username.clone().unwrap_or_default();
    if username.contains(':') {
        return Err(OptionRefusal::new("Username", NO_COLON));
    }
    let password = password
        .as_ref()
        .map(|password| password.0.expose_secret().to_owned())
        .unwrap_or_default();
    Ok(ProxyConfig {
        source,
        username,
        password: password.into(),
    })
}

/// `Basic` splits user and password at the first `:`, so one in the user moves the rest into the
/// password.
const NO_COLON: &str = "the username holds a `:`, which `Basic` authentication cannot carry";

/// How an `https://` endpoint is secured. Each file is read when the channel is created, so a
/// path that names nothing usable is refused then, by its option's name.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct TlsOptions {
    /// Which certificates the server's certificate is verified against, under the endpoint's host:
    /// that name is also the one sent as SNI.
    ///
    /// Defaults to `"System"`.
    #[serde(
        default,
        deserialize_with = "alternative::optional",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "ServerCertificates"))]
    pub server_certificates: Option<ServerCertificates>,

    /// The certificate the client presents, and its key.
    ///
    /// Defaults to `"None"`.
    #[serde(
        default,
        deserialize_with = "alternative::optional",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "ClientCertificate"))]
    pub client_certificate: Option<ClientCertificate>,
}

/// Which certificates the server's certificate is verified against.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum ServerCertificates {
    /// The system's roots.
    System,

    /// Against the roots of a PEM file, named by its path, in place of the system's. Every
    /// certificate the file holds is a root.
    CaPem(
        #[serde(deserialize_with = "within::non_empty")]
        #[cfg_attr(feature = "schema", schemars(length(min = 1)))]
        String,
    ),

    /// Against a root from a Windows certificate store, `Root` unless `Name` says otherwise, in
    /// place of the system's.
    ///
    /// Refused off Windows.
    CaStore(StoreCertificate),

    /// No verification: any server certificate is accepted. The connection is still encrypted,
    /// to whoever answers.
    None,
}

/// The certificate the client presents, and its key.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum ClientCertificate {
    /// No certificate is presented.
    None,

    /// From PEM files.
    Pem(PemCertificate),

    /// From a PKCS#12 bundle.
    P12(P12Certificate),

    /// From a Windows certificate store, `My` unless `Name` says otherwise, with the issuers the
    /// store's `CA` holds. Its key has to be exportable.
    ///
    /// Refused off Windows.
    Store(StoreCertificate),
}

/// A client certificate and its key, from PEM files.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct PemCertificate {
    /// Path to a PEM file of the client's certificate, then each issuer the server may not hold.
    #[cfg_attr(feature = "schema", schemars(length(min = 1)))]
    #[serde(deserialize_with = "within::non_empty")]
    pub certificate: String,

    /// Path to a PEM file of the certificate's key.
    #[cfg_attr(feature = "schema", schemars(length(min = 1)))]
    #[serde(deserialize_with = "within::non_empty")]
    pub key: String,
}

impl PemCertificate {
    pub fn new(certificate: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            certificate: certificate.into(),
            key: key.into(),
        }
    }

    fn load(&self) -> Result<ClientIdentity, OptionRefusal> {
        let chain = certificates("Certificate", &self.certificate)?;
        let key = PrivateKeyDer::from_pem_slice(&read("Key", &self.key)?).map_err(|error| {
            OptionRefusal::new(
                "Key",
                format!("the file it names holds no key PEM can carry: {error}"),
            )
        })?;
        Ok(ClientIdentity { chain, key })
    }
}

/// A client certificate and its key, from a PKCS#12 bundle.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct P12Certificate {
    /// Path to a PKCS#12 bundle of the client's certificate, the issuers it carries and the key.
    #[cfg_attr(feature = "schema", schemars(length(min = 1)))]
    #[serde(deserialize_with = "within::non_empty")]
    pub path: String,

    /// The password the bundle is protected by.
    ///
    /// Defaults to the empty one.
    #[serde(default, skip_serializing)]
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub password: Option<Password>,
}

impl P12Certificate {
    pub fn new(path: impl Into<String>, password: Option<Password>) -> Self {
        Self {
            path: path.into(),
            password,
        }
    }
}

/// A certificate of a Windows certificate store.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct StoreCertificate {
    /// Where the store is.
    ///
    /// Defaults to `CurrentUser`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "StoreLocation"))]
    pub location: Option<StoreLocation>,

    /// The store's name, such as `My`, `Root` or `CA`. Defaults to the one its option states.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "String", length(min = 1)))]
    #[serde(deserialize_with = "within::non_empty")]
    pub name: Option<String>,

    /// How the certificate is found in the store.
    #[serde(deserialize_with = "alternative::required")]
    pub find: StoreSearch,
}

impl StoreCertificate {
    pub fn new(find: StoreSearch) -> Self {
        Self {
            location: None,
            name: None,
            find,
        }
    }
}

/// Where a Windows certificate store is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum StoreLocation {
    /// The current user's stores.
    CurrentUser,

    /// The machine's stores, which every user shares.
    LocalMachine,
}

/// How a certificate is found in its store.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum StoreSearch {
    /// By its SHA-1 fingerprint, as 40 hexadecimal digits; spaces and colons between them are
    /// ignored.
    Thumbprint(
        #[serde(deserialize_with = "within::non_empty")]
        #[cfg_attr(feature = "schema", schemars(length(min = 1)))]
        String,
    ),

    /// By a text its subject contains, compared without case, as .NET's `FindBySubjectName`
    /// compares it.
    SubjectName(
        #[serde(deserialize_with = "within::non_empty")]
        #[cfg_attr(feature = "schema", schemars(length(min = 1)))]
        String,
    ),

    /// By its friendly name, exactly.
    FriendlyName(
        #[serde(deserialize_with = "within::non_empty")]
        #[cfg_attr(feature = "schema", schemars(length(min = 1)))]
        String,
    ),
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
            "Find.Thumbprint",
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
        if self.name.as_deref() == Some("") {
            return Err(OptionRefusal::new("Name", "it is empty"));
        }
        let local_machine = self.location == Some(StoreLocation::LocalMachine);
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
        let (by, key) = match &self.find {
            StoreSearch::Thumbprint(written) => {
                (By::Thumbprint(thumbprint(written)?), "Find.Thumbprint")
            }
            StoreSearch::SubjectName(subject) => (By::SubjectName(subject), "Find.SubjectName"),
            StoreSearch::FriendlyName(friendly) => {
                (By::FriendlyName(friendly), "Find.FriendlyName")
            }
        };
        // An empty text would match every certificate's subject, and pick one silently.
        if matches!(by, By::SubjectName("") | By::FriendlyName("")) {
            return Err(OptionRefusal::new(key, "it is empty"));
        }
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

/// The socket's keepalive.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum TcpKeepalive {
    /// No probe is sent.
    None,

    /// Probes the peer once the connection has been idle, and drops it when they go unanswered.
    Probe(TcpProbe),
}

/// The probes of a socket's keepalive, each duration a whole number of seconds, which is what the
/// socket option holds.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct TcpProbe {
    /// How many whole seconds the connection may be idle before the first probe, from 1 to 32767,
    /// the most Linux holds: the operating system counts whole seconds.
    #[cfg_attr(
        feature = "schema",
        schemars(with = "i32", range(min = 1, max = 32767))
    )]
    #[serde(deserialize_with = "within::between::<_, _, 1, 32767>")]
    pub idle_seconds: i32,

    /// How many whole seconds between two probes, from 1 to 32767. Defaults to the operating
    /// system's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "i32", range(min = 1, max = 32767))
    )]
    #[serde(deserialize_with = "within::between::<_, _, 1, 32767>")]
    pub interval_seconds: Option<i32>,

    /// How many probes go unanswered before the connection is dropped, at most 127, the most
    /// Linux holds. Defaults to the operating system's, and is not applied on Windows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "i32", range(min = 1, max = 127)))]
    #[serde(deserialize_with = "within::between::<_, _, 1, 127>")]
    pub retries: Option<i32>,
}

impl TcpProbe {
    /// A probe after `idle_seconds`, the interval and the count the operating system's.
    pub fn new(idle_seconds: i32) -> Self {
        Self {
            idle_seconds,
            interval_seconds: None,
            retries: None,
        }
    }
}

/// The HTTP/2 session a channel's calls share: how it checks that the peer is there, and how much
/// it lets the peer send ahead of what is read.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct Http2Options {
    /// Whether the session sends PINGs to check that the peer is there.
    ///
    /// Defaults to `"None"`: none is sent.
    #[serde(
        default,
        deserialize_with = "alternative::optional",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "Http2KeepAlive"))]
    pub keep_alive: Option<Http2KeepAlive>,

    /// How long a connection stays open with no call on it before it is closed, the next call
    /// dialling a new one. Each connection has its own. A call holds its connection to the end of
    /// its response and of its request.
    ///
    /// Defaults to `"None"`: an idle connection stays open.
    #[serde(
        default,
        deserialize_with = "alternative::optional",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "Http2IdleTimeout"))]
    pub idle_timeout: Option<Http2IdleTimeout>,

    /// How many calls one connection carries at once. A call that finds every connection full
    /// opens another, as many as the calls in flight need, and each closes on its own idle
    /// timeout when `IdleTimeout` is set.
    ///
    /// Defaults to `"FromServer"`.
    #[serde(
        default,
        deserialize_with = "alternative::optional",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "CallsPerConnection"))]
    pub simultaneous_calls_per_connection: Option<CallsPerConnection>,

    /// What the session sends.
    ///
    /// Defaults to `{}`, which leaves each of its options at its own default.
    #[serde(default)]
    pub send: Http2SendOptions,

    /// What the session lets the peer send.
    ///
    /// Defaults to `{"Fixed": {}}`: windows of 2 MiB per call and 5 MiB for the connection.
    #[serde(
        default,
        deserialize_with = "alternative::optional",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "Http2ReceiveOptions"))]
    pub receive: Option<Http2ReceiveOptions>,
}

/// Whether the session sends PINGs, which an unresponsive peer ends it for.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum Http2KeepAlive {
    /// No PING is sent.
    None,

    /// A PING is sent at an interval, and the session and its calls end when one goes unanswered.
    Ping(Http2Ping),
}

/// The PINGs of a session.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct Http2Ping {
    /// How often a PING is sent to the peer, at least a nanosecond.
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Seconds", extend("minimum" = 1e-9))
    )]
    #[serde(deserialize_with = "within::nanosecond")]
    pub interval_seconds: Seconds,

    /// How long a PING may go unanswered before the session and its calls are ended.
    ///
    /// Defaults to 20.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Seconds", extend("minimum" = 1e-9))
    )]
    #[serde(deserialize_with = "within::nanosecond")]
    pub timeout_seconds: Option<Seconds>,

    /// Whether a PING is also sent while no call is open.
    ///
    /// Defaults to false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "bool"))]
    pub while_idle: Option<bool>,
}

impl Http2Ping {
    /// A PING at an interval, with the default timeout and none sent while idle.
    pub fn new(interval_seconds: Seconds) -> Self {
        Self {
            interval_seconds,
            timeout_seconds: None,
            while_idle: None,
        }
    }
}

/// When a connection with no call on it is closed.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum Http2IdleTimeout {
    /// Never: an idle connection stays open.
    None,

    /// After this many seconds, at least a nanosecond.
    After(
        #[cfg_attr(
            feature = "schema",
            schemars(with = "Seconds", extend("minimum" = 1e-9))
        )]
        #[serde(deserialize_with = "within::nanosecond")]
        Seconds,
    ),
}

/// How many calls one connection carries at once.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum CallsPerConnection {
    /// As many as the server allows: the value of its SETTINGS_MAX_CONCURRENT_STREAMS.
    FromServer,

    /// At most this many, and never more than the server allows. At 1, calls follow one another
    /// on a connection but never share it, so that a GOAWAY a server sends because of one call -
    /// nginx's ENHANCE_YOUR_CALM against too many resets, for one - ends that call alone.
    Limit(
        #[serde(deserialize_with = "within::at_least::<_, _, 1>")]
        #[cfg_attr(feature = "schema", schemars(with = "i32", range(min = 1)))]
        i32,
    ),
}

/// What an HTTP/2 session sends.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct Http2SendOptions {
    /// How many bytes a write to the connection may gather before it goes. A write waits while
    /// the work already ready adds frames to it, one round of the runtime at a time, and goes once
    /// a round adds none or it holds this many bytes: a request's message handed over while its
    /// headers wait then goes out with them, in one write rather than two. 0 writes at once.
    ///
    /// Defaults to 16384, 16 KiB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "i32", range(min = 0)))]
    #[serde(deserialize_with = "within::at_least::<_, _, 0>")]
    pub coalescing_bytes: Option<i32>,

    /// How many KiB (1024 bytes) of one call's request may be queued in the session, waiting to be
    /// written, before its next part is handed over. A part is handed over whole once fewer bytes
    /// than this are queued, and the peer's window has room, so up to one part more than this is
    /// queued.
    ///
    /// Defaults to 1024, 1 MiB. At most 4194303, which is 4 GiB less a KiB: the session's
    /// buffer is counted in 32 bits.
    #[serde(
        default,
        rename = "StreamBufferKiB",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "i32", range(min = 1, max = LARGEST_STREAM_BUFFER_KIB))
    )]
    #[serde(deserialize_with = "within::stream_buffer_kib")]
    pub stream_buffer_kib: Option<i32>,

    /// How many DATA frames of the peer's largest size one queued part of a request may span,
    /// written one after the other in one write: a large message then goes out in fewer, larger
    /// writes. Above 1, a call reset while its part is being written can still send up to this
    /// many frames less one before its reset, and a PING or a SETTINGS acknowledgement queued
    /// behind DATA waits for this many times more of it. Above 1 needs an engine built against
    /// the h2-batch patch (`packages/rust/patches/h2-batch`), and is refused otherwise. At most
    /// 256.
    ///
    /// Defaults to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "i32", range(min = 1, max = LARGEST_FRAMES_PER_WRITE))
    )]
    #[serde(deserialize_with = "within::frames_per_write")]
    pub frames_per_write: Option<i32>,

    /// How many bytes the headers of one request may take. It bounds what is sent, never what is
    /// received.
    ///
    /// Defaults to `"Unbounded"`.
    #[serde(
        default,
        deserialize_with = "alternative::optional",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "HeaderListBytes"))]
    pub header_list_bytes: Option<HeaderListBytes>,
}

/// How many bytes the headers of one request may take.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum HeaderListBytes {
    /// No request is refused for its headers.
    Unbounded,

    /// At most this many bytes, counted as RFC 9113 counts a header list for
    /// SETTINGS_MAX_HEADER_LIST_SIZE: each field's name and value, and 32 more, the pseudo-header
    /// fields among them. A call whose request goes past it ends RESOURCE_EXHAUSTED before
    /// anything is sent.
    Max(
        #[serde(deserialize_with = "within::at_least::<_, _, 1>")]
        #[cfg_attr(feature = "schema", schemars(with = "i32", range(min = 1)))]
        i32,
    ),
}

/// What an HTTP/2 session lets its peer send ahead of what is read: windows of fixed sizes, or
/// windows that grow with what the link carries.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum Http2ReceiveOptions {
    /// Windows of fixed sizes, announced as the session opens.
    Fixed(Http2FixedWindows),

    /// Windows that grow with the link: both start at 65535, the size every connection starts
    /// with, and grow with the bandwidth-delay product the session's PINGs measure, up to 16 MiB.
    /// Neither shrinks.
    Adaptive,
}

/// HTTP/2 flow-control windows of fixed sizes.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct Http2FixedWindows {
    /// How many bytes of one call the peer may send ahead of what is read.
    ///
    /// Defaults to 2097152, 2 MiB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "i32", range(min = 1)))]
    #[serde(deserialize_with = "within::at_least::<_, _, 1>")]
    pub stream_window_bytes: Option<i32>,

    /// How many bytes the peer may send ahead of what is read, across every call of the channel.
    /// A call its host does not read holds up to `StreamWindowBytes` of it, so enough of them stop
    /// the others receiving. At least 65535, the window every connection starts with.
    ///
    /// Defaults to 5242880, 5 MiB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "i32", range(min = 65535)))]
    #[serde(deserialize_with = "within::at_least::<_, _, 65535>")]
    pub connection_window_bytes: Option<i32>,
}

/// What a channel does with the calls it sends: whether a failed call is sent again, whether the
/// channel slows down against a server that fails, and what it keeps of the messages for a call to
/// be sent again.
///
/// A failure is named by where it ended an attempt, in the entries of a list: `Status.X` for a
/// gRPC status the server sent in its trailers, X being a name from the gRPC specification such as
/// `UNAVAILABLE`; `Http.N` for an HTTP status N, from 100 to 599, that a proxy or a gateway answered
/// with and no gRPC status; `Reset.R` for a stream the server reset before its response, R being
/// an HTTP/2 error code from RFC 9113 such as `ENHANCE_YOUR_CALM`; `Pushback` for a failure whose
/// server asked for a wait in `grpc-retry-pushback-ms`, whatever its status; `Dial` for a connection
/// that could not be made; and `Connection` for one that ended under the call before the response.
/// An entry that names none of these is refused, and a list that is empty names nothing. What the
/// engine ended itself, a cancel and a deadline, is never a failure of the server's.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct OutboundTrafficOptions {
    /// Whether a failed call is sent again, and how.
    ///
    /// Defaults to `{"ExponentialBackoff": {}}`: five attempts in all, for what gRPC takes
    /// as `UNAVAILABLE`, a dial and a connection failure.
    #[serde(
        default,
        deserialize_with = "alternative::optional",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "RetryOptions"))]
    pub retry: Option<RetryOptions>,

    /// Whether the channel judges its server by what it accepts, and slows down against one that
    /// fails.
    ///
    /// Defaults to `{"Adaptive": {}}`: the estimate with every default.
    #[serde(
        default,
        deserialize_with = "alternative::optional",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "ThrottleOptions"))]
    pub throttle: Option<ThrottleOptions>,

    /// What the channel keeps of the messages its calls sent.
    ///
    /// Defaults to `{}`, which leaves each of its options at its own default.
    #[serde(default)]
    pub replay: ReplayOptions,
}

/// Whether a failed call is sent again, and how.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum RetryOptions {
    /// No retry: a failed call ends with its status. A call that its peer never processed still
    /// goes again, once for each way the peer did not see it, while what it sent is kept under
    /// `Replay`.
    None,

    /// A failed call is sent again, as gRFC A6 has it: after a backoff drawn below a bound that
    /// starts at `InitialBackoffSeconds` and grows by `BackoffMultiplier` to `MaxBackoffSeconds`,
    /// for the failures `FailureList` names, while no response head has reached the reader, what
    /// the call sent is still kept under `Replay`, and `Throttle` finds the server accepting what
    /// is sent. A policy that retries nothing is `None`, and neither a `MaxAttempts` of 1 nor an
    /// empty `FailureList` is one.
    ExponentialBackoff(ExponentialBackoffOptions),
}

impl Default for RetryOptions {
    fn default() -> Self {
        Self::ExponentialBackoff(ExponentialBackoffOptions::default())
    }
}

impl RetryOptions {
    /// The policy these options name, none when they name `None`, refused where they are
    /// incoherent.
    pub fn to_config(&self) -> Result<Option<RetryConfig>, OptionRefusal> {
        coherently(self.convert())
    }

    /// The policy, and the incoherences instead of a refusal for them.
    pub(crate) fn convert(&self) -> Converted<Option<RetryConfig>> {
        match self {
            Self::None => Ok((None, Vec::new())),
            Self::ExponentialBackoff(options) => {
                let (config, incoherent) = options
                    .convert()
                    .map_err(|refused| refused.under("ExponentialBackoff"))?;
                Ok((
                    Some(config),
                    incoherent
                        .into_iter()
                        .map(|refused| refused.under("ExponentialBackoff"))
                        .collect(),
                ))
            }
        }
    }
}

/// What a failed call is sent again by.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct ExponentialBackoffOptions {
    /// The failures a call is tried again for, each an entry as the options above describe.
    /// Empty retries nothing.
    ///
    /// Defaults to `["Status.UNAVAILABLE", "Http.502", "Http.503", "Http.504",
    /// "Reset.REFUSED_STREAM", "Dial", "Connection"]`: what gRPC takes as `UNAVAILABLE`, the status
    /// a proxy's 502, 503 and 504 and a refused stream map to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "Vec<String>"))]
    pub failure_list: Option<Vec<String>>,

    /// Attempts in all, the first included; at least 2, a policy that retries nothing being `None`.
    /// A call its peer never processed goes again besides, while every message it sent is kept.
    ///
    /// Defaults to 5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "i32", range(min = 2)))]
    #[serde(deserialize_with = "within::at_least::<_, _, 2>")]
    pub max_attempts: Option<i32>,

    /// The bound of the first backoff.
    ///
    /// Defaults to 5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Seconds", extend("minimum" = 1e-9))
    )]
    #[serde(deserialize_with = "within::nanosecond")]
    pub initial_backoff_seconds: Option<Seconds>,

    /// What the bound grows to and no further. Incoherent below `InitialBackoffSeconds`.
    ///
    /// Defaults to 120.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Seconds", extend("minimum" = 1e-9))
    )]
    #[serde(deserialize_with = "within::nanosecond")]
    pub max_backoff_seconds: Option<Seconds>,

    /// What each bound is multiplied by; 1 retries at a fixed bound.
    ///
    /// Defaults to 2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "f64", extend("minimum" = 1.0)))]
    #[serde(deserialize_with = "within::at_least::<_, _, 1>")]
    pub backoff_multiplier: Option<f64>,
}

/// The entries of a list of failures, each read as a cause.
///
/// A duplicate is let through once. `counted` is whether the list is one the estimate counts by,
/// which refuses what it never counts: the server's success, the caller's cancel and the caller's
/// own deadline.
fn causes(key: &str, entries: &[String], counted: bool) -> Result<Vec<Cause>, OptionRefusal> {
    let mut causes = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let cause = entry.parse::<Cause>().map_err(|unknown| {
            OptionRefusal::new(&format!("{key}[{index}]"), unknown.to_string())
        })?;
        if counted
            && matches!(
                cause,
                Cause::Status(GrpcStatusCode::Cancelled | GrpcStatusCode::DeadlineExceeded)
            )
        {
            return Err(OptionRefusal::new(
                &format!("{key}[{index}]"),
                format!("`{entry}` is never counted: it is the caller's own cancel or deadline"),
            ));
        }
        if !causes.contains(&cause) {
            causes.push(cause);
        }
    }
    Ok(causes)
}

impl ExponentialBackoffOptions {
    /// The policy these options name, each unset one at its default.
    pub fn to_config(&self) -> Result<RetryConfig, OptionRefusal> {
        coherently(self.convert())
    }

    /// The policy, and the incoherence instead of a refusal for it: the maximum backoff is then
    /// raised to the initial one.
    pub(crate) fn convert(&self) -> Converted<RetryConfig> {
        let defaults = RetryConfig::default();
        let initial_backoff = duration(
            "InitialBackoffSeconds",
            self.initial_backoff_seconds,
            1e-9,
            None,
        )?
        .unwrap_or(defaults.initial_backoff);
        let mut max_backoff = duration("MaxBackoffSeconds", self.max_backoff_seconds, 1e-9, None)?
            .unwrap_or(defaults.max_backoff);
        let mut incoherent = Vec::new();
        if max_backoff < initial_backoff {
            incoherent.push(OptionRefusal::incoherent(
                &["InitialBackoffSeconds", "MaxBackoffSeconds"],
                "the initial backoff is above the maximum, which the backoff grows to and no further",
            ));
            max_backoff = initial_backoff;
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
        let failures = match &self.failure_list {
            None => defaults.failures,
            Some(entries) => causes("FailureList", entries, false)?,
        };
        let max_attempts = match self.max_attempts {
            None => defaults.max_attempts,
            Some(value) if value >= 2 => value as u32,
            Some(value) => {
                return Err(OptionRefusal::new(
                    "MaxAttempts",
                    format!(
                        "{value} has to be at least 2; a source that wants no retry states `None`"
                    ),
                ))
            }
        };
        let config = RetryConfig {
            failures,
            max_attempts,
            initial_backoff,
            max_backoff,
            backoff_multiplier,
        };
        Ok((config, incoherent))
    }
}

/// Whether a channel slows down against a server that fails.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum ThrottleOptions {
    /// No judgment: every retry the retry policy chooses is sent, and first attempts start as they
    /// are made.
    None,

    /// An estimate of the server's health over a window of time, which sorts every attempt that
    /// ends as overloaded if `OverloadList` names its failure, as transient if `TransientList`
    /// does, and as accepted if it is an answer of the server's that neither names. Retries stop
    /// while the server fails more than `Multiplier` times what it accepts, beyond
    /// `FailureAllowance`. While it is overloaded more than `ThrottleMultiplier` times what it is
    /// not, beyond `FailureAllowance`, retries stop too, and first attempts start at a capped rate,
    /// and wait for their turns in the order they arrived: a call whose deadline passes while it waits ends
    /// `DEADLINE_EXCEEDED` having sent nothing. A transient failure, a server that is down, never
    /// slows first attempts. A deadline, a cancel, a GOAWAY and what the engine ended itself are
    /// never counted.
    Adaptive(AdaptiveOptions),
}

impl Default for ThrottleOptions {
    fn default() -> Self {
        Self::Adaptive(AdaptiveOptions::default())
    }
}

impl ThrottleOptions {
    /// The judgment these options name, none when they name `None`.
    pub fn to_config(&self) -> Result<Option<AdaptiveConfig>, OptionRefusal> {
        match self {
            Self::None => Ok(None),
            Self::Adaptive(options) => options
                .to_config()
                .map(Some)
                .map_err(|refused| refused.under("Adaptive")),
        }
    }
}

/// The estimate of a server's health, and what it does.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct AdaptiveOptions {
    /// The failures that may be an outage, each an entry as the options above describe. They slow
    /// retries and never lower the rate of first attempts. A failure of the server's that neither
    /// list names counts as an acceptance; `Status.CANCELLED` and `Status.DEADLINE_EXCEEDED` are
    /// refused.
    ///
    /// Defaults to `["Status.UNAVAILABLE", "Http.408", "Http.500", "Http.502", "Http.503",
    /// "Http.504", "Dial", "Connection"]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "Vec<String>"))]
    pub transient_list: Option<Vec<String>>,

    /// The failures that say the server is over capacity, each an entry as the options above
    /// describe. They slow retries and lower the rate of first attempts. A failure that both
    /// lists name is overload.
    ///
    /// Defaults to `["Status.RESOURCE_EXHAUSTED", "Http.429", "Pushback", "Reset.ENHANCE_YOUR_CALM",
    /// "Reset.REFUSED_STREAM"]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "Vec<String>"))]
    pub overload_list: Option<Vec<String>>,

    /// How many times what the server accepts the channel may send, as retries stop: they are open
    /// while the attempts that ended, less this many times the accepted ones, are at most
    /// `FailureAllowance`. At least 1 and at most 100.
    ///
    /// Defaults to 2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "f64", extend("minimum" = 1.0, "maximum" = 100.0))
    )]
    #[serde(deserialize_with = "within::between::<_, _, 1, 100>")]
    pub multiplier: Option<f64>,

    /// How many times what the server does not report as overloaded the channel may send, as the
    /// rate is capped: the cap is on while the attempts that ended, less this many times those not
    /// overloaded, are over `FailureAllowance`. At least 1 and at most 100.
    ///
    /// Defaults to 2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "f64", extend("minimum" = 1.0, "maximum" = 100.0))
    )]
    #[serde(deserialize_with = "within::between::<_, _, 1, 100>")]
    pub throttle_multiplier: Option<f64>,

    /// The failures beyond the multiple of what the server accepts that are let go, so that a
    /// channel with little traffic does not lose its retries, or its rate, to one failure.
    ///
    /// Defaults to 10.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "i32", range(min = 0, max = 1000000))
    )]
    #[serde(deserialize_with = "within::between::<_, _, 0, 1000000>")]
    pub failure_allowance: Option<i32>,

    /// How far back the counts reach, from 0.012 to 600 seconds.
    ///
    /// Defaults to 30.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Seconds", extend("minimum" = 0.012, "maximum" = 600.0))
    )]
    #[serde(deserialize_with = "within::adaptive_window_seconds")]
    pub window_seconds: Option<Seconds>,

    /// The rate of first attempts, a second, that the cap never goes under, so that the channel goes
    /// on probing a server that is overloaded. Above 0 and at most 1000000.
    ///
    /// Defaults to 0.5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "f64", extend("exclusiveMinimum" = 0.0, "maximum" = 1000000.0))
    )]
    #[serde(deserialize_with = "within::floor_per_second")]
    pub floor_per_second: Option<f64>,
}

impl AdaptiveOptions {
    /// The judgment these options name, each unset one at its default.
    pub fn to_config(&self) -> Result<AdaptiveConfig, OptionRefusal> {
        let defaults = AdaptiveConfig::default();
        let multiplier = |key: &str, asked: Option<f64>, default: f64| match asked {
            None => Ok(default),
            Some(value) if value.is_finite() && (1.0..=100.0).contains(&value) => Ok(value),
            Some(value) => Err(OptionRefusal::new(
                key,
                format!("{value} has to be a number from 1 to 100"),
            )),
        };
        let slack = match self.failure_allowance {
            None => defaults.slack,
            Some(value) if (0..=1_000_000).contains(&value) => value as u32,
            Some(value) => {
                return Err(OptionRefusal::new(
                    "FailureAllowance",
                    format!("{value} has to be between 0 and 1000000"),
                ))
            }
        };
        let floor_per_second = match self.floor_per_second {
            None => defaults.floor_per_second,
            Some(value) if value.is_finite() && value > 0.0 && value <= 1_000_000.0 => value,
            Some(value) => {
                return Err(OptionRefusal::new(
                    "FloorPerSecond",
                    format!("{value} has to be above 0 and at most 1000000"),
                ))
            }
        };
        let list = |key: &str, entries: &Option<Vec<String>>, default: Vec<Cause>| match entries {
            None => Ok(default),
            Some(entries) => causes(key, entries, true),
        };
        Ok(AdaptiveConfig {
            transient: list("TransientList", &self.transient_list, defaults.transient)?,
            overload: list("OverloadList", &self.overload_list, defaults.overload)?,
            multiplier: multiplier("Multiplier", self.multiplier, defaults.multiplier)?,
            throttle_multiplier: multiplier(
                "ThrottleMultiplier",
                self.throttle_multiplier,
                defaults.throttle_multiplier,
            )?,
            slack,
            window: duration("WindowSeconds", self.window_seconds, 0.012, Some(600.0))?
                .unwrap_or(defaults.window),
            floor_per_second,
        })
    }
}

/// What a channel keeps of the messages its calls sent, so that a call can be sent again: after a
/// failure the retry policy chose, and when its peer never processed it.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct ReplayOptions {
    /// The KiB one call may keep; a call that sends more is not sent again.
    ///
    /// Defaults to 1024, 1 MiB.
    #[serde(
        default,
        rename = "MaxPerCallKiB",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "i32", range(min = 0)))]
    #[serde(deserialize_with = "within::at_least::<_, _, 0>")]
    pub max_per_call_kib: Option<i32>,

    /// The KiB all of the channel's calls may keep together; a call whose message would pass it is
    /// not sent again.
    ///
    /// Defaults to 16384, 16 MiB.
    #[serde(
        default,
        rename = "MaxPerChannelKiB",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "i32", range(min = 0)))]
    #[serde(deserialize_with = "within::at_least::<_, _, 0>")]
    pub max_per_channel_kib: Option<i32>,
}

impl ReplayOptions {
    /// What these options keep, each unset one at its default.
    pub fn to_config(&self) -> Result<ReplayConfig, OptionRefusal> {
        let defaults = ReplayConfig::default();
        let bytes = |key: &str, asked: Option<i32>, default: usize| match asked {
            None => Ok(default),
            Some(kib) if kib < 0 => Err(OptionRefusal::new(
                key,
                format!("{kib} has to be at least 0"),
            )),
            Some(kib) => usize::try_from(i64::from(kib) * 1024).map_err(|_| {
                OptionRefusal::new(key, format!("{kib} KiB are more than this platform holds"))
            }),
        };
        Ok(ReplayConfig {
            call_bytes: bytes("MaxPerCallKiB", self.max_per_call_kib, defaults.call_bytes)?,
            channel_bytes: bytes(
                "MaxPerChannelKiB",
                self.max_per_channel_kib,
                defaults.channel_bytes,
            )?,
        })
    }
}

/// An option refused, named by its path from the unit that read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionRefusal {
    key: String,
    /// The other keys of an incoherence, which names the options that cannot all hold.
    with: Vec<String>,
    incoherent: bool,
    why: String,
}

impl OptionRefusal {
    fn new(key: &str, why: impl Into<String>) -> Self {
        Self {
            key: key.to_owned(),
            with: Vec::new(),
            incoherent: false,
            why: why.into(),
        }
    }

    /// Options that cannot all hold once the options are merged: each is valid, the set of them
    /// is not. A channel's options with one are refused, and a runtime's defaults with one are
    /// said, since a channel can still state another.
    fn incoherent(keys: &[&str], why: impl Into<String>) -> Self {
        Self {
            key: keys[0].to_owned(),
            with: keys[1..].iter().map(|key| (*key).to_owned()).collect(),
            incoherent: true,
            why: why.into(),
        }
    }

    /// The same refusal, named from the unit `unit` sits in: a unit does not know where it is
    /// embedded, so the embedding adds its own name.
    pub fn under(self, unit: &str) -> Self {
        Self {
            key: format!("{unit}.{}", self.key),
            with: self
                .with
                .into_iter()
                .map(|key| format!("{unit}.{key}"))
                .collect(),
            incoherent: self.incoherent,
            why: self.why,
        }
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    /// The keys the refusal names: one, or the several of an incoherence.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.key.as_str()).chain(self.with.iter().map(String::as_str))
    }

    /// Whether the options are each valid and cannot hold together, as opposed to one being wrong
    /// by itself.
    #[cfg(test)]
    pub(crate) fn is_incoherence(&self) -> bool {
        self.incoherent
    }
}

impl std::fmt::Display for OptionRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.incoherent {
            let keys: Vec<&str> = self.keys().collect();
            let (last, first) = keys.split_last().expect("an incoherence names its keys");
            if first.is_empty() {
                write!(f, "{last} is incoherent: {}", self.why)
            } else {
                write!(
                    f,
                    "{} and {last} are incoherent: {}",
                    first.join(", "),
                    self.why
                )
            }
        } else {
            write!(f, "{} is refused: {}", self.key, self.why)
        }
    }
}

/// What a unit's options became, and the incoherences among them. A unit that is incoherent still
/// has a config, which leaves out or adjusts what cannot hold together, so that the caller may use
/// it and say the incoherences.
pub(crate) type Converted<C> = Result<(C, Vec<OptionRefusal>), OptionRefusal>;

/// A conversion that has to be coherent: the first incoherence is the refusal.
fn coherently<C>(converted: Converted<C>) -> Result<C, OptionRefusal> {
    let (config, incoherent) = converted?;
    match incoherent.into_iter().next() {
        Some(first) => Err(first),
        None => Ok(config),
    }
}

impl std::error::Error for OptionRefusal {}

/// A number of seconds as a duration, refused below `least`, above `most`, and past what a
/// `Duration` holds; none when no number is stated.
fn duration(
    key: &str,
    seconds: Option<Seconds>,
    least: f64,
    most: Option<f64>,
) -> Result<Option<Duration>, OptionRefusal> {
    seconds
        .map(|seconds| stated_duration(key, seconds, least, most))
        .transpose()
}

/// A stated number of seconds as a duration, refused as `duration` refuses.
fn stated_duration(
    key: &str,
    seconds: Seconds,
    least: f64,
    most: Option<f64>,
) -> Result<Duration, OptionRefusal> {
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
    Duration::try_from(seconds).map_err(|_| refused())
}

/// A size in KiB as the bytes it is, refused below 1. A size past what an address holds is the
/// largest one.
fn kibibytes(key: &str, kib: i32) -> Result<usize, OptionRefusal> {
    if kib < 1 {
        return Err(OptionRefusal::new(
            key,
            format!("{kib} has to be at least 1"),
        ));
    }
    Ok((kib as usize).saturating_mul(1024))
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
        let (roots, accept_any_server) = match &self.server_certificates {
            None | Some(ServerCertificates::System) => (Vec::new(), false),
            Some(ServerCertificates::CaPem(path)) => {
                (certificates("ServerCertificates.CaPem", path)?, false)
            }
            Some(ServerCertificates::CaStore(store)) => (
                vec![store
                    .root()
                    .map_err(|refused| refused.under("ServerCertificates.CaStore"))?],
                false,
            ),
            Some(ServerCertificates::None) => (Vec::new(), true),
        };

        let identity = match &self.client_certificate {
            None | Some(ClientCertificate::None) => None,
            Some(ClientCertificate::Pem(pem)) => Some(
                pem.load()
                    .map_err(|refused| refused.under("ClientCertificate.Pem"))?,
            ),
            Some(ClientCertificate::P12(p12)) => Some(
                pkcs12("Path", &p12.path, p12.password.as_ref())
                    .map_err(|refused| refused.under("ClientCertificate.P12"))?,
            ),
            Some(ClientCertificate::Store(store)) => Some(
                store
                    .identity()
                    .map_err(|refused| refused.under("ClientCertificate.Store"))?,
            ),
        };

        Ok(TlsConfig {
            roots,
            accept_any_server,
            identity,
        })
    }
}

impl TcpKeepalive {
    /// The socket's keepalive this names, none for `None`.
    pub fn to_config(&self) -> Result<TcpConfig, OptionRefusal> {
        match self {
            Self::None => Ok(TcpConfig::default()),
            Self::Probe(probe) => probe.to_config().map_err(|refused| refused.under("Probe")),
        }
    }
}

impl TcpProbe {
    fn to_config(&self) -> Result<TcpConfig, OptionRefusal> {
        let whole = |key: &str, seconds: i32| {
            if (1..=32767).contains(&seconds) {
                Ok(Duration::from_secs(seconds as u64))
            } else {
                Err(OptionRefusal::new(
                    key,
                    format!("{seconds} has to be from 1 to 32767"),
                ))
            }
        };
        let retries = match self.retries {
            None => None,
            Some(retries) if !(1..=127).contains(&retries) => {
                return Err(OptionRefusal::new(
                    "Retries",
                    format!("{retries} has to be from 1 to 127"),
                ))
            }
            Some(retries) => Some(retries as u32),
        };
        Ok(TcpConfig {
            keepalive: Some(whole("IdleSeconds", self.idle_seconds)?),
            keepalive_interval: self
                .interval_seconds
                .map(|seconds| whole("IntervalSeconds", seconds))
                .transpose()?,
            keepalive_retries: retries,
        })
    }
}

impl Http2Ping {
    /// The interval, the timeout and whether to ping while idle, each unstated one `defaults`'.
    fn to_config(
        &self,
        defaults: &Http2Config,
    ) -> Result<(Option<Duration>, Duration, bool), OptionRefusal> {
        let interval = stated_duration("IntervalSeconds", self.interval_seconds, 1e-9, None)?;
        let timeout = duration("TimeoutSeconds", self.timeout_seconds, 1e-9, None)?
            .unwrap_or(defaults.keep_alive_timeout);
        Ok((
            Some(interval),
            timeout,
            self.while_idle.unwrap_or(defaults.keep_alive_while_idle),
        ))
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
        let (keep_alive_interval, keep_alive_timeout, keep_alive_while_idle) =
            match &self.keep_alive {
                None | Some(Http2KeepAlive::None) => (
                    None,
                    defaults.keep_alive_timeout,
                    defaults.keep_alive_while_idle,
                ),
                Some(Http2KeepAlive::Ping(ping)) => ping
                    .to_config(&defaults)
                    .map_err(|refused| refused.under("KeepAlive.Ping"))?,
            };
        Ok(Http2Config {
            keep_alive_interval,
            keep_alive_timeout,
            keep_alive_while_idle,
            receive_windows: match &self.receive {
                None => defaults.receive_windows,
                Some(Http2ReceiveOptions::Fixed(windows)) => {
                    let fixed = FixedWindows::default();
                    ReceiveWindows::Fixed(FixedWindows {
                        stream: window(
                            "Receive.Fixed.StreamWindowBytes",
                            windows.stream_window_bytes,
                            1,
                            fixed.stream,
                        )?,
                        connection: window(
                            "Receive.Fixed.ConnectionWindowBytes",
                            windows.connection_window_bytes,
                            65_535,
                            fixed.connection,
                        )?,
                    })
                }
                Some(Http2ReceiveOptions::Adaptive) => ReceiveWindows::Adaptive,
            },
            idle_timeout: match &self.idle_timeout {
                None | Some(Http2IdleTimeout::None) => None,
                Some(Http2IdleTimeout::After(seconds)) => {
                    Some(stated_duration("IdleTimeout.After", *seconds, 1e-9, None)?)
                }
            },
            simultaneous_calls_per_connection: match &self.simultaneous_calls_per_connection {
                None | Some(CallsPerConnection::FromServer) => None,
                Some(CallsPerConnection::Limit(calls)) if *calls < 1 => {
                    return Err(OptionRefusal::new(
                        "SimultaneousCallsPerConnection.Limit",
                        format!("{calls} has to be at least 1"),
                    ))
                }
                Some(CallsPerConnection::Limit(calls)) => Some(*calls as usize),
            },
            write_coalescing: match self.send.coalescing_bytes {
                None => defaults.write_coalescing,
                Some(bytes) if bytes < 0 => {
                    return Err(OptionRefusal::new(
                        "Send.CoalescingBytes",
                        format!("{bytes} has to be at least 0"),
                    ))
                }
                Some(bytes) => bytes as usize,
            },
            send_buffer: match self.send.stream_buffer_kib {
                None => defaults.send_buffer,
                Some(kib) if kib > LARGEST_STREAM_BUFFER_KIB => {
                    return Err(OptionRefusal::new(
                        "Send.StreamBufferKiB",
                        format!("{kib} has to be at most {LARGEST_STREAM_BUFFER_KIB}"),
                    ))
                }
                Some(kib) => kibibytes("Send.StreamBufferKiB", kib)?,
            },
            frames_per_write: match self.send.frames_per_write {
                None => defaults.frames_per_write,
                Some(frames) if !(1..=LARGEST_FRAMES_PER_WRITE as i32).contains(&frames) => {
                    return Err(OptionRefusal::new(
                        "Send.FramesPerWrite",
                        format!("{frames} has to be between 1 and {LARGEST_FRAMES_PER_WRITE}"),
                    ))
                }
                Some(frames) if !cfg!(h2_batch) && frames > 1 => {
                    return Err(OptionRefusal::new(
                        "Send.FramesPerWrite",
                        format!(
                            "{frames} needs an engine built against packages/rust/patches/h2-batch"
                        ),
                    ))
                }
                Some(frames) => frames as usize,
            },
            max_header_list_size: match &self.send.header_list_bytes {
                None | Some(HeaderListBytes::Unbounded) => None,
                Some(HeaderListBytes::Max(size)) if *size < 1 => {
                    return Err(OptionRefusal::new(
                        "Send.HeaderListBytes.Max",
                        format!("{size} has to be at least 1"),
                    ))
                }
                Some(HeaderListBytes::Max(size)) => Some(*size as usize),
            },
        })
    }
}

/// What a caller may set on one channel.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct ChannelOptions {
    /// What the transport does, beyond reaching the endpoint.
    ///
    /// Defaults to `{}`, which leaves each of its options at its own default.
    #[serde(default)]
    pub transport: TransportOptions,

    /// The HTTP/2 session the channel's calls share.
    ///
    /// Defaults to `{}`, which leaves each of its options at its own default.
    #[serde(default)]
    pub http2: Http2Options,

    /// What the channel's calls do: their messages, deadlines and retries, and what crosses
    /// between the host and the engine.
    ///
    /// Defaults to `{}`, which leaves each of its options at its own default.
    #[serde(default)]
    pub grpc: GrpcOptions,
}

/// What the channel's calls do: their messages, deadlines and retries, and what crosses between
/// the host and the engine.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct GrpcOptions {
    /// What this client calls itself in `user-agent`.
    ///
    /// Defaults to `armonik-transport/` followed by the engine's version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "String", length(min = 1)))]
    #[serde(deserialize_with = "within::non_empty")]
    pub user_agent: Option<String>,

    /// The deadline of a call that states none.
    ///
    /// Defaults to `"None"`, a call waiting as long as its answer takes.
    #[serde(
        default,
        deserialize_with = "alternative::optional",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "Deadline"))]
    pub deadline: Option<Deadline>,

    /// What the channel does with the calls it sends: sending a failed one again, slowing down
    /// against a server that fails, and keeping messages for a call to be sent again.
    ///
    /// Defaults to `{}`, which leaves each of its options at its own default.
    #[serde(default)]
    pub outbound_traffic: OutboundTrafficOptions,

    /// What a call sends to the server.
    ///
    /// Defaults to `{}`, which leaves each of its options at its own default.
    #[serde(default)]
    pub send: GrpcSendOptions,

    /// What a call accepts from the server.
    ///
    /// Defaults to `{}`, which leaves each of its options at its own default.
    #[serde(default)]
    pub receive: GrpcReceiveOptions,

    /// What crosses between the host and the engine on each call.
    ///
    /// Defaults to `{}`, which leaves each of its options at its own default.
    #[serde(default)]
    pub host: HostOptions,
}

/// The deadline of a call that states none, counted from its start.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum Deadline {
    /// The call has none: it waits as long as its answer takes.
    None,

    /// The call ends `DEADLINE_EXCEEDED` once this many seconds have passed, at least a
    /// nanosecond, the finest duration the engine holds, and the server is told what was left of
    /// it when the call started as `grpc-timeout`. It bounds the whole call, a streaming one
    /// included, and not only the wait for the response's head. A call's own deadline takes its
    /// place.
    Default(
        #[cfg_attr(
            feature = "schema",
            schemars(with = "Seconds", extend("minimum" = 1e-9))
        )]
        #[serde(deserialize_with = "within::nanosecond")]
        Seconds,
    ),
}

/// An encoding the messages of a call may be compressed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum MessageEncoding {
    /// RFC 1952 gzip, `gzip` on the wire.
    Gzip,
    /// gRPC's `deflate`: the zlib structure of RFC 1950 around an RFC 1951 stream, and not a raw
    /// RFC 1951 stream.
    Deflate,
    /// RFC 8878 Zstandard, `zstd` on the wire.
    Zstd,
}

impl MessageEncoding {
    /// The engine's encoding of the same name.
    pub fn encoding(self) -> crate::grpc::Encoding {
        match self {
            Self::Gzip => crate::grpc::Encoding::Gzip,
            Self::Deflate => crate::grpc::Encoding::Deflate,
            Self::Zstd => crate::grpc::Encoding::Zstd,
        }
    }
}

/// How the messages a call sends are compressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum SendCompression {
    /// No compression, `identity` on the wire: the messages go out as they are and no
    /// `grpc-encoding` is sent.
    None,
    /// RFC 1952 gzip, `gzip` on the wire.
    Gzip,
    /// gRPC's `deflate`: the zlib structure of RFC 1950 around an RFC 1951 stream, and not a raw
    /// RFC 1951 stream.
    Deflate,
    /// RFC 8878 Zstandard, `zstd` on the wire.
    Zstd,
}

impl SendCompression {
    /// The engine's encoding of the same name, none for `None`.
    pub fn encoding(self) -> Option<crate::grpc::Encoding> {
        match self {
            Self::None => None,
            Self::Gzip => Some(crate::grpc::Encoding::Gzip),
            Self::Deflate => Some(crate::grpc::Encoding::Deflate),
            Self::Zstd => Some(crate::grpc::Encoding::Zstd),
        }
    }
}

impl GrpcReceiveOptions {
    /// The encodings this client accepts besides `identity`, in the order stated: `identity` is
    /// always accepted, and an empty list says that there is no other.
    pub fn accepted_encodings(&self) -> Vec<crate::grpc::Encoding> {
        self.compression
            .iter()
            .flatten()
            .map(|encoding| encoding.encoding())
            .collect()
    }
}

/// What a call sends to the server.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct GrpcSendOptions {
    /// The largest message this client will send. A larger one ends its call
    /// `RESOURCE_EXHAUSTED`, and none of it is sent.
    ///
    /// Defaults to `"Unbounded"`, any message a call is given going out.
    #[serde(
        default,
        rename = "MessageSizeKiB",
        deserialize_with = "alternative::optional",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "SendMessageSizeKiB"))]
    pub message_size_kib: Option<SendMessageSizeKiB>,

    /// The encoding the messages of a call are compressed with, which the call states as
    /// `grpc-encoding`. A message that would not be smaller compressed is sent as it is, and
    /// `MessageSizeKiB` is checked on a message before it is compressed.
    ///
    /// The server has to accept the encoding, and says which it accepts in the
    /// `grpc-accept-encoding` of its responses. A response that lists encodings without this one
    /// stops the channel compressing: the calls that start after it send their messages as they
    /// are, and the channel logs a warning once. A later response that lists it has the channel
    /// compress again. A call that reached a server which does not accept the encoding ends
    /// `UNIMPLEMENTED` and is not sent again.
    ///
    /// Defaults to `"None"`, the messages going out as they are.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "SendCompression"))]
    pub compression: Option<SendCompression>,
}

/// The largest message a call sends.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum SendMessageSizeKiB {
    /// No message is refused for its size.
    Unbounded,

    /// At most this many KiB (1024 bytes), counted before the message is compressed.
    Max(
        #[serde(deserialize_with = "within::at_least::<_, _, 1>")]
        #[cfg_attr(feature = "schema", schemars(with = "i32", range(min = 1)))]
        i32,
    ),
}

/// The largest message a call accepts.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum ReceiveMessageSizeKiB {
    /// No message is refused for its size.
    Unbounded,

    /// At most this many KiB (1024 bytes), counted once the message is decompressed.
    Max(
        #[serde(deserialize_with = "within::at_least::<_, _, 1>")]
        #[cfg_attr(feature = "schema", schemars(with = "i32", range(min = 1)))]
        i32,
    ),
}

impl SendMessageSizeKiB {
    /// The limit in bytes, none for no limit.
    pub fn limit(&self) -> Result<Option<usize>, OptionRefusal> {
        match self {
            Self::Unbounded => Ok(None),
            Self::Max(kib) => kibibytes("Max", *kib).map(Some),
        }
    }
}

impl ReceiveMessageSizeKiB {
    /// The limit in bytes, the largest there is for no limit.
    pub fn limit(&self) -> Result<usize, OptionRefusal> {
        match self {
            Self::Unbounded => Ok(usize::MAX),
            Self::Max(kib) => kibibytes("Max", *kib),
        }
    }
}

/// What a call accepts from the server.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct GrpcReceiveOptions {
    /// The largest message this client will accept.
    ///
    /// Defaults to 4096, 4 MiB, as `{"Max": 4096}`.
    #[serde(
        default,
        rename = "MessageSizeKiB",
        deserialize_with = "alternative::optional",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "ReceiveMessageSizeKiB"))]
    pub message_size_kib: Option<ReceiveMessageSizeKiB>,

    /// The encodings besides `identity` that this client accepts for the messages of an answer,
    /// which it states as `grpc-accept-encoding` in the order given, `identity` last. A server
    /// may then compress what it sends, in the first of them that it knows. A name given twice
    /// counts at its first place. `MessageSizeKiB` bounds a message once it is decompressed. A
    /// message compressed in an encoding that is not listed ends its call `INTERNAL`. `identity`
    /// is always accepted.
    ///
    /// Defaults to none, only `identity` being accepted, which an empty list says too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "Vec<MessageEncoding>"))]
    pub compression: Option<Vec<MessageEncoding>>,
}

/// What crosses between the host and the engine on each call, one way and the other.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct HostOptions {
    /// What the host sends.
    ///
    /// Defaults to `{}`, which leaves each of its options at its own default.
    #[serde(default)]
    pub send: HostSendOptions,

    /// What the engine delivers to the host.
    ///
    /// Defaults to `{}`, which leaves each of its options at its own default.
    #[serde(default)]
    pub receive: HostReceiveOptions,
}

/// What a call's host sends: the messages it hands the engine.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct HostSendOptions {
    /// How many messages a call may have sent and unacquitted at once.
    ///
    /// Defaults to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "i32", range(min = 1, max = LARGEST_WINDOW))
    )]
    #[serde(deserialize_with = "within::window")]
    pub window: Option<i32>,
}

/// What the engine delivers to a call's host: its payloads and its status.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct HostReceiveOptions {
    /// How many of a call's payloads the host may hold at once, delivered and not yet given back.
    /// The terminal status takes none, so a host holds at most one more.
    ///
    /// Defaults to 4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "i32", range(min = 1, max = LARGEST_WINDOW))
    )]
    #[serde(deserialize_with = "within::window")]
    pub window: Option<i32>,

    /// How many bytes of a response a delivery to the host may wait to gather, so that a unary
    /// answer's head, message and status reach it in one callback. 0 delivers each read at once.
    ///
    /// Defaults to 16384, 16 KiB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "i32", range(min = 0)))]
    #[serde(deserialize_with = "within::at_least::<_, _, 0>")]
    pub coalescing_bytes: Option<i32>,
}

/// Options stated over their defaults, by the shape of the type alone. A struct with only optional
/// fields merges field by field, recursively, and an option is the default's where it is not
/// stated. A struct with a mandatory field is stated whole: it replaces the default's, its optional
/// fields taking what it states or their default, so that no source leaves it half stated. An
/// alternative stated over the same variant merges what the two carry by that rule, and over
/// another variant is taken whole, so two alternatives are never combined into one neither stated.
trait Over {
    /// Whether a struct holding this as a field has it mandatory: so of a value and of a struct
    /// with a mandatory field of its own, not of an `Option` or of a struct with none.
    fn mandatory(&self) -> bool {
        true
    }

    fn over(self, defaults: &Self) -> Self;
}

impl<T: Over + Clone> Over for Option<T> {
    fn mandatory(&self) -> bool {
        false
    }

    fn over(self, defaults: &Self) -> Self {
        match (self, defaults) {
            (Some(own), Some(default)) => Some(own.over(default)),
            (Some(own), None) => Some(own),
            (None, default) => default.clone(),
        }
    }
}

/// `Over` for a value, which a stated one replaces whole.
macro_rules! over_values {
    ($($type:ty),+ $(,)?) => {
        $(
            impl Over for $type {
                fn over(self, _: &Self) -> Self {
                    self
                }
            }
        )+
    };
}

over_values!(
    String,
    i32,
    u64,
    f64,
    bool,
    Seconds,
    Password,
    StoreLocation,
    MessageEncoding,
    SendCompression,
    Vec<MessageEncoding>,
    CredentialedUrl,
    Vec<String>,
);

/// `Over` for an enum of alternatives: a variant that carries nothing is taken whole, the same
/// variant that carries a value merges what the two carry, and another is taken whole. Every
/// variant is listed and matched without `_`, so a variant the enum gains and this does not list
/// fails to compile - and so is the list of names [`alternative`] reads a name against.
macro_rules! over_variants {
    ($type:ident { $($unit:ident),* ; $($variant:ident),* $(,)? }) => {
        impl Over for $type {
            fn over(self, defaults: &Self) -> Self {
                match self {
                    $(Self::$unit => Self::$unit,)*
                    $(Self::$variant(own) => Self::$variant(match defaults {
                        Self::$variant(default) => own.over(default),
                        _ => own,
                    }),)*
                }
            }
        }

        impl alternative::Alternative for $type {
            const NAME: &'static str = stringify!($type);
            const VARIANTS: &'static [&'static str] =
                &[$(stringify!($unit),)* $(stringify!($variant)),*];
        }
    };
}

/// The bounds the schema states, checked where a value is read: a document out of one is refused by
/// its key's path, as one of the wrong type is, rather than when its options become a config. Each
/// is a field's `deserialize_with`, so the field keeps the one type the schema is rendered from.
mod within {
    use serde::de::{Deserialize, Deserializer, Error};

    use super::{Seconds, LARGEST_FRAMES_PER_WRITE, LARGEST_STREAM_BUFFER_KIB, LARGEST_WINDOW};

    /// What a bound is held against: an integer, exactly, or a number.
    pub(super) enum Measure {
        Integer(i128),
        Number(f64),
    }

    impl Measure {
        fn at_least(&self, least: i64) -> bool {
            match self {
                Self::Integer(value) => *value >= i128::from(least),
                Self::Number(value) => *value >= least as f64,
            }
        }

        fn at_most(&self, most: i64) -> bool {
            match self {
                Self::Integer(value) => *value <= i128::from(most),
                Self::Number(value) => *value <= most as f64,
            }
        }

        fn number(&self) -> f64 {
            match self {
                Self::Integer(value) => *value as f64,
                Self::Number(value) => *value,
            }
        }
    }

    /// A value a bound applies to, or none when an option is left out.
    pub(super) trait Measured {
        fn measured(&self) -> Option<Measure>;
    }

    macro_rules! integers {
        ($($integer:ty),+) => {
            $(
                impl Measured for $integer {
                    fn measured(&self) -> Option<Measure> {
                        Some(Measure::Integer(i128::from(*self)))
                    }
                }
            )+
        };
    }

    integers!(i32, u64);

    impl Measured for f64 {
        fn measured(&self) -> Option<Measure> {
            Some(Measure::Number(*self))
        }
    }

    impl Measured for Seconds {
        fn measured(&self) -> Option<Measure> {
            Some(Measure::Number(self.0))
        }
    }

    impl<T: Measured> Measured for Option<T> {
        fn measured(&self) -> Option<Measure> {
            self.as_ref().and_then(Measured::measured)
        }
    }

    /// A text that can be empty, which a bound of one character refuses; an option left out is
    /// not.
    pub(super) trait Textual {
        fn is_empty(&self) -> bool;
    }

    impl Textual for String {
        fn is_empty(&self) -> bool {
            self.as_str().is_empty()
        }
    }

    impl<T: Textual> Textual for Option<T> {
        fn is_empty(&self) -> bool {
            self.as_ref().is_some_and(Textual::is_empty)
        }
    }

    /// The value read as its type reads it, then held to `holds`, and refused with `says` - not
    /// the value - when it does not. A number that is not finite is refused first, with a message
    /// of its own, but for a `Seconds`, which its own reader has refused by then.
    fn checked<'de, D, T>(
        deserializer: D,
        holds: impl Fn(&Measure) -> bool,
        says: std::fmt::Arguments<'_>,
    ) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de> + Measured,
    {
        let value = T::deserialize(deserializer)?;
        match value.measured() {
            Some(Measure::Number(number)) if !number.is_finite() => {
                Err(D::Error::custom("it has to be a finite number"))
            }
            Some(measure) if !holds(&measure) => Err(D::Error::custom(says)),
            _ => Ok(value),
        }
    }

    /// At least `MIN`.
    pub(super) fn at_least<'de, D, T, const MIN: i64>(deserializer: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de> + Measured,
    {
        checked(
            deserializer,
            |measure| measure.at_least(MIN),
            format_args!("it has to be at least {MIN}"),
        )
    }

    /// From `MIN` to `MAX`.
    pub(super) fn between<'de, D, T, const MIN: i64, const MAX: i64>(
        deserializer: D,
    ) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de> + Measured,
    {
        checked(
            deserializer,
            |measure| measure.at_least(MIN) && measure.at_most(MAX),
            format_args!("it has to be between {MIN} and {MAX}"),
        )
    }

    /// At least a nanosecond, the finest duration the engine holds.
    pub(super) fn nanosecond<'de, D, T>(deserializer: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de> + Measured,
    {
        checked(
            deserializer,
            |measure| measure.number() >= 1e-9,
            format_args!("it has to be at least a nanosecond, 1e-9"),
        )
    }

    /// From 0.012 to 600 seconds.
    pub(super) fn adaptive_window_seconds<'de, D, T>(deserializer: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de> + Measured,
    {
        checked(
            deserializer,
            |measure| (0.012..=600.0).contains(&measure.number()),
            format_args!("it has to be between 0.012 and 600"),
        )
    }

    /// Above 0 and at most 1000000.
    pub(super) fn floor_per_second<'de, D, T>(deserializer: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de> + Measured,
    {
        checked(
            deserializer,
            |measure| measure.number() > 0.0 && measure.number() <= 1_000_000.0,
            format_args!("it has to be above 0 and at most 1000000"),
        )
    }

    /// From 1 to the most KiB a stream's send buffer may hold.
    pub(super) fn stream_buffer_kib<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<i32>, D::Error> {
        between::<D, Option<i32>, 1, { LARGEST_STREAM_BUFFER_KIB as i64 }>(deserializer)
    }

    /// From 1 to the most frames one write may span.
    pub(super) fn frames_per_write<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<i32>, D::Error> {
        between::<D, Option<i32>, 1, { LARGEST_FRAMES_PER_WRITE as i64 }>(deserializer)
    }

    /// From 1 to the deepest window either side of a call may be given.
    pub(super) fn window<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<i32>, D::Error> {
        between::<D, Option<i32>, 1, { LARGEST_WINDOW as i64 }>(deserializer)
    }

    /// A text of at least one character.
    pub(super) fn non_empty<'de, D, T>(deserializer: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de> + Textual,
    {
        let value = T::deserialize(deserializer)?;
        if value.is_empty() {
            Err(D::Error::custom(
                "it has to be a text of at least one character",
            ))
        } else {
            Ok(value)
        }
    }
}

/// How an alternative is read: by the name of a variant that carries nothing, or by an object whose
/// one key names a variant and holds what it carries, as serde reads an externally tagged enum.
///
/// By hand rather than by serde's derive, which refuses an object of no key: that one is no
/// alternative stated, and keeps what an earlier source gave it. A key that names no variant is
/// refused, and so is a name that is none.
mod alternative {
    use std::marker::PhantomData;

    use serde::de::value::EnumAccessDeserializer;
    use serde::de::{
        self, DeserializeOwned, DeserializeSeed, Deserializer, EnumAccess, IntoDeserializer,
        MapAccess, VariantAccess, Visitor,
    };

    /// An enum read as an alternative, by the names of its variants.
    pub(super) trait Alternative: DeserializeOwned {
        const NAME: &'static str;
        const VARIANTS: &'static [&'static str];
    }

    /// An alternative that may be left out, and is none when it is an object of no key.
    pub(super) fn optional<'de, D: Deserializer<'de>, T: Alternative>(
        deserializer: D,
    ) -> Result<Option<T>, D::Error> {
        deserializer.deserialize_option(Optional(PhantomData))
    }

    /// An alternative a document has to state, refused when it is an object of no key.
    pub(super) fn required<'de, D: Deserializer<'de>, T: Alternative>(
        deserializer: D,
    ) -> Result<T, D::Error> {
        deserializer
            .deserialize_enum(T::NAME, T::VARIANTS, Chosen(PhantomData))?
            .ok_or_else(|| {
                de::Error::custom(format_args!(
                    "it names none of {}, and one is needed",
                    T::VARIANTS.join(", ")
                ))
            })
    }

    struct Optional<T>(PhantomData<T>);

    impl<'de, T: Alternative> Visitor<'de> for Optional<T> {
        type Value = Option<T>;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "one of {}", T::VARIANTS.join(", "))
        }

        fn visit_none<E: de::Error>(self) -> Result<Option<T>, E> {
            Ok(None)
        }

        fn visit_unit<E: de::Error>(self) -> Result<Option<T>, E> {
            Ok(None)
        }

        fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Option<T>, D::Error> {
            deserializer.deserialize_enum(T::NAME, T::VARIANTS, Chosen(PhantomData))
        }
    }

    /// The variant a name or an object's key gives, if one does.
    struct Chosen<T>(PhantomData<T>);

    impl<'de, T: Alternative> Visitor<'de> for Chosen<T> {
        type Value = Option<T>;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "one of {}", T::VARIANTS.join(", "))
        }

        /// A name, or an object of one key.
        fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<Option<T>, A::Error> {
            T::deserialize(EnumAccessDeserializer::new(data)).map(Some)
        }

        /// An object of no key or of several, which a loader hands over as it reads it: the
        /// variant its key names, if it has one; two are refused.
        fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Option<T>, M::Error> {
            let mut chosen = None;
            while let Some(key) = map.next_key::<String>()? {
                if chosen.is_some() {
                    return Err(de::Error::custom(
                        "it names two alternatives, of which one is chosen at a time",
                    ));
                }
                chosen = Some(map.next_value_seed(Named::<T> {
                    name: key,
                    kind: PhantomData,
                })?);
            }
            Ok(chosen)
        }
    }

    /// The variant `name`, read from the value its key holds.
    struct Named<T> {
        name: String,
        kind: PhantomData<T>,
    }

    impl<'de, T: Alternative> DeserializeSeed<'de> for Named<T> {
        type Value = T;

        fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<T, D::Error> {
            T::deserialize(OneVariant {
                name: self.name,
                value: deserializer,
            })
        }
    }

    /// An enum of one variant whose value is `value`, which the derived reader takes as it takes
    /// any enum.
    struct OneVariant<D> {
        name: String,
        value: D,
    }

    impl<'de, D: Deserializer<'de>> Deserializer<'de> for OneVariant<D> {
        type Error = D::Error;

        fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, D::Error> {
            visitor.visit_enum(self)
        }

        serde::forward_to_deserialize_any! {
            bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf
            option unit unit_struct newtype_struct seq tuple tuple_struct map struct enum
            identifier ignored_any
        }
    }

    impl<'de, D: Deserializer<'de>> EnumAccess<'de> for OneVariant<D> {
        type Error = D::Error;
        type Variant = Content<D>;

        fn variant_seed<V: DeserializeSeed<'de>>(
            self,
            seed: V,
        ) -> Result<(V::Value, Content<D>), D::Error> {
            let name = seed.deserialize(self.name.into_deserializer())?;
            Ok((name, Content(self.value)))
        }
    }

    struct Content<D>(D);

    impl<'de, D: Deserializer<'de>> VariantAccess<'de> for Content<D> {
        type Error = D::Error;

        fn unit_variant(self) -> Result<(), D::Error> {
            <() as de::Deserialize>::deserialize(self.0)
        }

        fn newtype_variant_seed<T: DeserializeSeed<'de>>(
            self,
            seed: T,
        ) -> Result<T::Value, D::Error> {
            seed.deserialize(self.0)
        }

        fn tuple_variant<V: Visitor<'de>>(
            self,
            len: usize,
            visitor: V,
        ) -> Result<V::Value, D::Error> {
            self.0.deserialize_tuple(len, visitor)
        }

        fn struct_variant<V: Visitor<'de>>(
            self,
            fields: &'static [&'static str],
            visitor: V,
        ) -> Result<V::Value, D::Error> {
            self.0.deserialize_struct("", fields, visitor)
        }
    }
}

over_variants!(ServerCertificates {
    System,
    None;
    CaPem,
    CaStore,
});
over_variants!(ClientCertificate {
    None;
    Pem,
    P12,
    Store,
});
over_variants!(ProxyOptions {
    None;
    System,
    Url,
    UrlWithCredentials,
});
over_variants!(StoreSearch {
    ;
    Thumbprint,
    SubjectName,
    FriendlyName,
});

/// `Over` for a struct of options: every field merged, or the struct stated whole when one field is
/// mandatory. The fields are destructured without `..`, so a field the struct gains and this does
/// not list fails to compile.
macro_rules! over_fields {
    ($type:ident { $($field:ident),+ $(,)? }) => {
        impl Over for $type {
            fn mandatory(&self) -> bool {
                [$(self.$field.mandatory()),+].into_iter().any(|stated| stated)
            }

            fn over(self, defaults: &Self) -> Self {
                if self.mandatory() {
                    return self;
                }
                let Self { $($field),+ } = self;
                Self {
                    $($field: $field.over(&defaults.$field)),+
                }
            }
        }
    };
}

over_fields!(ChannelOptions {
    transport,
    http2,
    grpc,
});
over_fields!(TransportOptions {
    connect_timeout_seconds,
    tls,
    tcp_keepalive,
    proxy,
    connect_eagerly,
});
over_variants!(Deadline {
    None;
    Default,
});
over_variants!(SendMessageSizeKiB {
    Unbounded;
    Max,
});
over_variants!(ReceiveMessageSizeKiB {
    Unbounded;
    Max,
});
over_fields!(GrpcOptions {
    user_agent,
    deadline,
    outbound_traffic,
    send,
    receive,
    host,
});
over_fields!(GrpcSendOptions {
    message_size_kib,
    compression,
});
over_fields!(GrpcReceiveOptions {
    message_size_kib,
    compression,
});
over_fields!(HostOptions { send, receive });
over_fields!(HostSendOptions { window });
over_fields!(HostReceiveOptions {
    window,
    coalescing_bytes,
});
over_fields!(TlsOptions {
    server_certificates,
    client_certificate,
});
over_variants!(TcpKeepalive {
    None;
    Probe,
});
over_fields!(TcpProbe {
    idle_seconds,
    interval_seconds,
    retries,
});
over_variants!(Http2KeepAlive {
    None;
    Ping,
});
over_fields!(Http2Ping {
    interval_seconds,
    timeout_seconds,
    while_idle,
});
over_variants!(Http2IdleTimeout {
    None;
    After,
});
over_variants!(CallsPerConnection {
    FromServer;
    Limit,
});
over_fields!(Http2Options {
    keep_alive,
    idle_timeout,
    simultaneous_calls_per_connection,
    send,
    receive,
});
over_variants!(HeaderListBytes {
    Unbounded;
    Max,
});
over_fields!(Http2SendOptions {
    coalescing_bytes,
    stream_buffer_kib,
    frames_per_write,
    header_list_bytes,
});
over_fields!(Http2FixedWindows {
    stream_window_bytes,
    connection_window_bytes,
});
over_variants!(Http2ReceiveOptions {
    Adaptive;
    Fixed,
});
over_variants!(RetryOptions {
    None;
    ExponentialBackoff,
});
over_fields!(OutboundTrafficOptions {
    retry,
    throttle,
    replay,
});
over_fields!(ExponentialBackoffOptions {
    failure_list,
    max_attempts,
    initial_backoff_seconds,
    max_backoff_seconds,
    backoff_multiplier,
});
over_variants!(ThrottleOptions {
    None;
    Adaptive,
});
over_fields!(AdaptiveOptions {
    transient_list,
    overload_list,
    multiplier,
    throttle_multiplier,
    failure_allowance,
    window_seconds,
    floor_per_second,
});
over_fields!(ReplayOptions {
    max_per_call_kib,
    max_per_channel_kib,
});
over_fields!(PemCertificate { certificate, key });

over_fields!(ProxyCredentials { username, password });
over_fields!(ProxyUrl {
    address,
    username,
    password,
});
over_fields!(P12Certificate { path, password });
over_fields!(StoreCertificate {
    location,
    name,
    find,
});

impl ChannelOptions {
    /// Refuses what these options state that is wrong by itself, whatever they are merged over:
    /// an alternative whose own values contradict it, such as a `Url` whose address carries
    /// credentials. Run on a document before it is merged.
    pub fn check(&self) -> Result<(), OptionRefusal> {
        match &self.transport.proxy {
            Some(proxy) => proxy
                .to_config()
                .map(drop)
                .map_err(|refused| refused.under("Transport.Proxy")),
            None => Ok(()),
        }
    }

    /// These options over `defaults`: field by field, recursively, an option stated here winning
    /// and one left out the default's, but for a group of options with a mandatory field, such as
    /// a `Probe`, which is stated whole and replaces the default's. An alternative - how the server
    /// is verified, who the client is, which proxy - stated over the same one merges as its
    /// payload does, and over another is taken whole. Options that only bound one another, such as
    /// the two backoff bounds, merge as any option, and a merge where they disagree is refused as a
    /// document stating both would be.
    pub fn over(self, defaults: &Self) -> Self {
        Over::over(self, defaults)
    }
}

/// What a caller may set on the runtime: the endpoint, the memory ceiling, the options every
/// channel takes where its own state none, and what the engine logs.
#[derive(Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct RuntimeOptions {
    /// The server, as `http://host:port` in the clear or `https://host:port` over TLS, that a
    /// channel created with no endpoint of its own reaches.
    ///
    /// Defaults to none: every channel then names its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "String", length(min = 1)))]
    #[serde(deserialize_with = "within::non_empty")]
    pub endpoint: Option<String>,

    /// The memory the runtime holds, in two thresholds.
    ///
    /// Defaults to `{}`, which leaves each of its options at its own default.
    #[serde(default)]
    pub memory_ceiling: MemoryCeilingOptions,

    /// Channel options every channel of the runtime takes where its own options state none: the
    /// two are merged option by option, a struct's options within it, and the channel's win; a
    /// group of options with a mandatory field, such as a `Probe`, is stated whole and replaces the
    /// default's. An alternative - how the server is verified, who the client is, which proxy -
    /// merges as its payload does over the same alternative and is taken whole over another.
    ///
    /// Defaults to none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "ChannelOptions"))]
    pub channel_defaults: Option<ChannelOptions>,

    /// What the engine reports of itself to the host.
    ///
    /// Defaults to `{}`, which leaves each of its options at its own default.
    #[serde(default)]
    pub logging: LoggingOptions,
}

/// The memory the runtime holds: the bytes counting the buffers lent to send, the messages
/// received until the host gives them back and the compressed copies of sent messages while they
/// are held, and where work waits and where the runtime stops.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct MemoryCeilingOptions {
    /// The MiB the runtime holds before work waits: a call stops reading, and a send waits for
    /// room. A compressed copy that would pass it is dropped instead, and its message goes out
    /// uncompressed.
    ///
    /// Defaults to 4096, four gigabytes, or 2048 where half the address space is smaller; a larger
    /// value is that too.
    #[serde(default, rename = "SoftMiB", skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "i64", range(min = 1)))]
    #[serde(deserialize_with = "within::between::<_, _, 1, 9223372036854775807>")]
    pub soft_mib: Option<u64>,

    /// The MiB past which the runtime stops: a received message that would take the count past
    /// them ends its call with RESOURCE_EXHAUSTED. Calls admitted to read below `SoftMiB` may pass
    /// it together, by a message each, and this bounds them. At least `SoftMiB`, or its default
    /// when that is left out, which is checked once the options are merged.
    ///
    /// Defaults to a quarter above `SoftMiB`.
    #[serde(default, rename = "HardMiB", skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "i64", range(min = 1)))]
    #[serde(deserialize_with = "within::between::<_, _, 1, 9223372036854775807>")]
    pub hard_mib: Option<u64>,
}

impl MemoryCeilingOptions {
    /// The default of `SoftMiB`, where the address space holds four gigabytes.
    pub const DEFAULT_SOFT_MIB: u64 = if usize::BITS >= 64 { 4096 } else { 2048 };

    /// Refuses what the options cannot hold, once they are merged: a ceiling of 0, and a hard
    /// ceiling below the soft one. Nothing states another option after the runtime's, so the
    /// incoherence is an error.
    pub fn check(&self) -> Result<(), OptionRefusal> {
        self.unqualified_check()
            .map_err(|refused| refused.under("MemoryCeiling"))
    }

    fn unqualified_check(&self) -> Result<(), OptionRefusal> {
        for (key, mib) in [("SoftMiB", self.soft_mib), ("HardMiB", self.hard_mib)] {
            if mib == Some(0) {
                return Err(OptionRefusal::new(key, "it is 0, and has to be at least 1"));
            }
        }
        let soft = self.soft_mib.unwrap_or(Self::DEFAULT_SOFT_MIB);
        match self.hard_mib {
            Some(hard) if hard < soft => Err(OptionRefusal::incoherent(
                &["SoftMiB", "HardMiB"],
                format!("the hard ceiling is {hard} MiB, below the soft one, {soft} MiB"),
            )),
            _ => Ok(()),
        }
    }

    /// The soft and the hard ceiling in bytes, 0 where the library's own is meant. A count past
    /// what 64 bits hold is the largest one.
    pub fn bytes(&self) -> (u64, u64) {
        let bytes = |mib: Option<u64>| mib.map_or(0, |mib| mib.saturating_mul(1 << 20));
        (bytes(self.soft_mib), bytes(self.hard_mib))
    }
}

/// Which of the engine's log events a host receives.
///
/// Ignored by a Rust host.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct LoggingOptions {
    /// Which events are reported: comma-separated directives, each a level for every target
    /// (`warn`, or `*=warn`), a target and its level (`h2=debug`), or a target alone, which is all
    /// its levels. A target covers itself and the modules below it - `h2` covers `h2::proto`, not
    /// `h2x` - and `target*` covers every target that starts with the text: `hyper*` covers
    /// `hyper` and `hyper_util`. The most specific directive that covers an event decides: the
    /// longest target, and at the same length the one without `*`. A filter replaces the default
    /// whole, and a target none of its directives covers is off: `armonik_transport=debug` alone
    /// reports that target and nothing else, `*=off` alone reports nothing, and a word that is no
    /// level is a target nothing emits. A level for every target, as `*=warn` states it, covers
    /// the rest. A directive that is not understood is ignored with a warning, and a filter with
    /// none that is understood, an empty one included, is the default. Read when the runtime is
    /// created.
    ///
    /// Defaults to `*=warn,armonik_transport*=info`: warnings from every target, and the engine's
    /// own events - the targets that start with `armonik_transport` - at information.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub filter: Option<String>,
}

/// The endpoint is printed elided, since it may carry a password.
impl std::fmt::Debug for RuntimeOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeOptions")
            .field("endpoint", &self.endpoint.as_deref().map(elided))
            .field("memory_ceiling", &self.memory_ceiling)
            .field("channel_defaults", &self.channel_defaults)
            .field("logging", &self.logging)
            .finish()
    }
}

over_fields!(MemoryCeilingOptions { soft_mib, hard_mib });
over_fields!(RuntimeOptions {
    endpoint,
    memory_ceiling,
    channel_defaults,
    logging,
});
over_fields!(LoggingOptions { filter });

impl crate::configuration::Document for ChannelOptions {
    fn over(self, earlier: Self) -> Self {
        Over::over(self, &earlier)
    }
}

impl crate::configuration::Document for RuntimeOptions {
    fn over(self, earlier: Self) -> Self {
        Over::over(self, &earlier)
    }
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
    rendered::<ChannelOptions>()
}

/// The schema of [`RuntimeOptions`], as the committed `runtime.schema.json` holds it, rendered as
/// [`schema`] renders the channel's.
#[cfg(feature = "schema")]
pub fn runtime_schema() -> String {
    rendered::<RuntimeOptions>()
}

#[cfg(feature = "schema")]
fn rendered<T: schemars::JsonSchema>() -> String {
    let schema = schemars::generate::SchemaSettings::default()
        .with_transform(schemars::transform::RecursiveTransform(
            |schema: &mut schemars::Schema| {
                schema.remove("default");
            },
        ))
        .into_generator()
        .into_root_schema_for::<T>();
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
            server_certificates: Some(ServerCertificates::CaPem(write(
                &directory,
                "ca.pem",
                &certificate,
            ))),
            client_certificate: Some(ClientCertificate::Pem(PemCertificate::new(
                write(&directory, "chain.pem", &two),
                write(&directory, "key.pem", &key),
            ))),
        };

        let config = options.load().expect("readable files");
        assert_eq!(config.roots.len(), 1);
        let identity = config.identity.expect("an identity");
        assert_eq!(
            identity.chain.len(),
            2,
            "the whole chain, in the file's order"
        );
        assert!(!config.accept_any_server);

        let unverified = TlsOptions {
            server_certificates: Some(ServerCertificates::None),
            ..TlsOptions::default()
        }
        .load()
        .expect("nothing to read");
        assert!(unverified.accept_any_server);
        assert!(unverified.roots.is_empty());
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
        let pem = |certificate: &str, key: &str| TlsOptions {
            client_certificate: Some(ClientCertificate::Pem(PemCertificate::new(
                certificate,
                key,
            ))),
            ..TlsOptions::default()
        };

        for (options, key) in [
            (
                TlsOptions {
                    server_certificates: Some(ServerCertificates::CaPem(missing.clone())),
                    ..TlsOptions::default()
                },
                "ServerCertificates.CaPem",
            ),
            (
                TlsOptions {
                    server_certificates: Some(ServerCertificates::CaPem(empty.clone())),
                    ..TlsOptions::default()
                },
                "ServerCertificates.CaPem",
            ),
            (
                pem(&missing, &certificate),
                "ClientCertificate.Pem.Certificate",
            ),
            (pem(&certificate, &certificate), "ClientCertificate.Pem.Key"),
            (pem(&certificate, &missing), "ClientCertificate.Pem.Key"),
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

    /// A variant that carries nothing is its name, and one that carries something is an object of
    /// one key: written so, and read so by serde itself and by the loader.
    #[test]
    fn a_variant_that_carries_nothing_is_a_string_and_the_others_an_object() {
        let proxy = |proxy: ProxyOptions| TransportOptions {
            proxy: Some(proxy),
            ..TransportOptions::default()
        };
        let written = |proxy: &TransportOptions| serde_json::to_string(proxy).expect("a document");
        assert_eq!(
            written(&proxy(ProxyOptions::None)),
            r#"{"Tls":{},"Proxy":"None"}"#
        );
        assert_eq!(
            written(&proxy(ProxyOptions::UrlWithCredentials(CredentialedUrl(
                "http://p".to_owned()
            )))),
            r#"{"Tls":{},"Proxy":{"UrlWithCredentials":"http://p"}}"#
        );

        let read = |document: &str| serde_json::from_str::<TransportOptions>(document);
        assert_eq!(
            read(r#"{"Proxy":"None"}"#).expect("a name").proxy,
            Some(ProxyOptions::None)
        );
        assert_eq!(
            read(r#"{"Proxy":{"None":null}}"#).expect("an object").proxy,
            Some(ProxyOptions::None)
        );
        for refused in [
            r#"{"Proxy":{"None":true}}"#,
            r#"{"Proxy":"Url"}"#,
            r#"{"Proxy":"Socks"}"#,
            r#"{"Proxy":"none"}"#,
            r#"{"Proxy":{"Socks":{"Address":"x"}}}"#,
        ] {
            assert!(read(refused).is_err(), "{refused}");
        }
    }

    /// Alternatives exclude one another by their shape: a document naming two is refused as it
    /// is read, before any file is.
    #[test]
    fn a_document_naming_two_alternatives_is_refused() {
        for document in [
            r#"{"ServerCertificates":{"CaPem":"ca.pem","None":null}}"#,
            r#"{"ClientCertificate":{"Pem":{"Certificate":"c.pem","Key":"k.pem"},"P12":{"Path":"c.p12"}}}"#,
            r#"{"ServerCertificates":{"None":true}}"#,
            r#"{"ServerCertificates":{"None":false}}"#,
            r#"{"ServerCertificates":"CaPem"}"#,
            r#"{"ServerCertificates":"Elsewhere"}"#,
            r#"{"ClientCertificate":{"Pem":{"Certificate":"c.pem"}}}"#,
            r#"{"ClientCertificate":{"P12":{"Password":"s3cret"}}}"#,
        ] {
            let read = serde_json::from_str::<TlsOptions>(document);
            assert!(read.is_err(), "{document}");
            assert!(
                !read.unwrap_err().to_string().contains("s3cret"),
                "{document}"
            );
        }
        let read: TlsOptions = serde_json::from_str(
            r#"{"ServerCertificates":"None","ClientCertificate":{"P12":{"Path":"c.p12","Password":"x"}}}"#,
        )
        .expect("one alternative each");
        assert_eq!(read.server_certificates, Some(ServerCertificates::None));
        assert_eq!(
            read.client_certificate,
            Some(ClientCertificate::P12(P12Certificate::new(
                "c.p12",
                Some(Password::new("x"))
            )))
        );
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

    fn p12(path: &str, password: Option<&str>) -> TlsOptions {
        TlsOptions {
            client_certificate: Some(ClientCertificate::P12(P12Certificate::new(
                path,
                password.map(Password::new),
            ))),
            ..TlsOptions::default()
        }
    }

    #[test]
    fn a_pkcs12_bundle_is_read_into_the_identity_it_carries() {
        let directory = scratch("p12");
        let _gone = Scratch(directory.clone());
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(["client.test".to_owned()]).expect("an identity");

        let options = p12(
            &bundled(
                &directory,
                "identity.p12",
                p12_bundle(&signing_key, &[&cert], "s3cret-word"),
            ),
            Some("s3cret-word"),
        );
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

        let unprotected = p12(
            &bundled(
                &directory,
                "open.p12",
                p12_bundle(&signing_key, &[&cert], ""),
            ),
            None,
        );
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

        for options in [
            p12(&protected, Some("hunter2")),
            p12(&empty, Some("s3cret-word")),
            p12(&garbage, None),
            p12(&two, Some("s3cret-word")),
        ] {
            let refused = options.load().expect_err("refused");
            assert_eq!(refused.key(), "ClientCertificate.P12.Path", "{refused}");
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
        let store = StoreCertificate::new(StoreSearch::FriendlyName("anything".to_owned()));
        for (options, unit) in [
            (
                TlsOptions {
                    client_certificate: Some(ClientCertificate::Store(store.clone())),
                    ..TlsOptions::default()
                },
                "ClientCertificate.Store",
            ),
            (
                TlsOptions {
                    server_certificates: Some(ServerCertificates::CaStore(store.clone())),
                    ..TlsOptions::default()
                },
                "ServerCertificates.CaStore",
            ),
        ] {
            let refused = options.load().expect_err(unit);
            assert!(refused.key().starts_with(unit), "{refused}");
            assert!(refused.to_string().contains("Windows only"), "{refused}");
        }
    }

    fn url(address: &str, username: Option<&str>, password: Option<&str>) -> ProxyOptions {
        let mut url = ProxyUrl::new(address);
        url.username = username.map(str::to_owned);
        url.password = password.map(Password::new);
        ProxyOptions::Url(url)
    }

    fn system(username: Option<&str>, password: Option<&str>) -> ProxyOptions {
        ProxyOptions::System(ProxyCredentials {
            username: username.map(str::to_owned),
            password: password.map(Password::new),
        })
    }

    #[test]
    fn a_proxy_url_becomes_the_proxy_tunnelled_through_with_its_credentials() {
        let explicit = |config: ProxyConfig| match config.source {
            ProxySource::Explicit(uri) => (uri.to_string(), config.username, config.password),
            other => panic!("{other:?}"),
        };

        let (uri, username, password) = explicit(
            url("proxy.test:3128", Some("alice"), Some("s3cret"))
                .to_config()
                .expect("a proxy"),
        );
        assert_eq!(uri, "http://proxy.test:3128/", "http:// is assumed");
        assert_eq!(
            (username.as_str(), password.expose_secret()),
            ("alice", "s3cret")
        );

        let (uri, username, password) = explicit(
            ProxyOptions::UrlWithCredentials(CredentialedUrl(
                "http://alice:s%40cret@proxy.test:3128".to_owned(),
            ))
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

        let config = ProxyOptions::None.to_config().expect("no proxy");
        assert_eq!(config.source, ProxySource::Disabled);

        let (uri, _, _) = explicit(
            url("http://[::1]:3128", None, None)
                .to_config()
                .expect("a bracketed IPv6 proxy"),
        );
        assert_eq!(uri, "http://[::1]:3128/");

        let refused = url("proxy.test:3128", Some("corp:alice"), Some("s3cret"))
            .to_config()
            .expect_err("a `:` in the username");
        assert_eq!(refused.key(), "Url.Username");
    }

    #[test]
    fn the_system_proxy_is_the_default_and_takes_the_dedicated_credentials() {
        assert_eq!(ProxyOptions::default(), system(None, None));
        let config = system(Some("alice"), None)
            .to_config()
            .expect("the environment's proxy");
        assert_eq!(config.source, ProxySource::System);
        assert_eq!(config.username, "alice");
        assert_eq!(config.password.expose_secret(), "");
        let refused = system(Some("corp:alice"), None)
            .to_config()
            .expect_err("a `:` in the username");
        assert_eq!(refused.key(), "System.Username");
    }

    /// The system proxy's credentials have no mandatory field, so they merge field by field over
    /// the defaults, as any group of options does.
    #[test]
    fn the_system_proxys_credentials_merge_field_by_field() {
        let defaults = system(Some("alice"), Some("s3cret"));
        assert_eq!(system(None, None).over(&defaults), defaults);
        assert_eq!(
            system(Some("bob"), None).over(&defaults),
            system(Some("bob"), Some("s3cret"))
        );
        assert_eq!(
            system(None, Some("other")).over(&defaults),
            system(Some("alice"), Some("other"))
        );
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
            let printed = format!("{:?}", url(address, None, None));
            assert!(printed.contains(shown), "{address}: {printed}");
            assert!(!printed.contains("s3cret"), "{printed}");
        }
    }

    #[test]
    fn a_proxy_refusal_names_the_address_and_quotes_neither_it_nor_a_password() {
        for options in [
            url("https://proxy.test:443", None, None),
            url("http://alice:s3cret@proxy.test", Some("bob"), None),
            url("http://alice:s3cret@proxy.test", None, Some("other")),
            url("http://proxy.test:99999", None, None),
            url("http://proxy.test:s3cret", None, None),
            url("http://:3128", None, None),
            url("not a url", None, None),
            url("http://proxy.test:3128/pac.js", None, None),
            url("http://proxy.test:3128/?s3cret", None, None),
            url("http://proxy.test:3128#s3cret", None, None),
            url("http://alice@proxy.test:3128", None, None),
        ] {
            let refused = options.to_config().expect_err("refused");
            assert_eq!(refused.key(), "Url.Address", "{options:?}: {refused}");
            let said = refused.to_string();
            assert!(!said.contains("s3cret"), "{said}");
            assert!(!said.contains("proxy.test"), "{said}");
            assert!(!format!("{options:?}").contains("s3cret"), "{options:?}");
        }
    }

    /// A URL carrying its credentials is refused by its own option when it carries none, does not
    /// decode, puts a `:` in the username or is no `http://` URL; neither the refusal nor a Debug
    /// print quotes it.
    #[test]
    fn a_credentialed_url_refusal_quotes_neither_it_nor_its_password() {
        let credentialed =
            |address: &str| ProxyOptions::UrlWithCredentials(CredentialedUrl(address.to_owned()));
        for options in [
            credentialed("http://proxy.test:3128"),
            credentialed("https://alice:s3cret@proxy.test:443"),
            credentialed("http://alice:%FF@proxy.test:3128"),
            credentialed("http://corp%3Aalice:s3cret@proxy.test:3128"),
            credentialed("not a url"),
        ] {
            let refused = options.to_config().expect_err("refused");
            assert_eq!(refused.key(), "UrlWithCredentials", "{refused}");
            let said = refused.to_string();
            assert!(!said.contains("s3cret"), "{said}");
            assert!(!said.contains("proxy.test"), "{said}");
            assert!(!format!("{options:?}").contains("s3cret"), "{options:?}");
        }
    }

    /// A document's proxy is checked alone, so a URL that carries its credentials in the wrong
    /// alternative is refused by the document's own path.
    #[test]
    fn a_document_is_checked_alone_by_its_own_paths() {
        let options = ChannelOptions {
            transport: TransportOptions {
                proxy: Some(url("http://alice:s3cret@proxy.test:3128", None, None)),
                ..TransportOptions::default()
            },
            ..ChannelOptions::default()
        };
        let refused = options.check().expect_err("credentials in a Url");
        assert_eq!(refused.key(), "Transport.Proxy.Url.Address");
        assert!(ChannelOptions::default().check().is_ok());
    }

    #[test]
    fn a_unit_refusal_is_named_from_where_the_unit_is_embedded() {
        let refused = TcpKeepalive::Probe(TcpProbe {
            retries: Some(0),
            ..TcpProbe::new(30)
        })
        .to_config()
        .expect_err("no probe may go unanswered zero times")
        .under("Transport.TcpKeepalive");
        assert_eq!(refused.key(), "Transport.TcpKeepalive.Probe.Retries");
        assert!(!refused.is_incoherence());
        assert!(!refused.to_string().contains("  "), "{refused}");
        assert!(refused
            .to_string()
            .starts_with("Transport.TcpKeepalive.Probe.Retries is refused"));
    }

    #[test]
    fn the_keepalive_options_become_the_socket_configuration() {
        let config = TcpKeepalive::Probe(TcpProbe {
            idle_seconds: 30,
            interval_seconds: Some(5),
            retries: Some(3),
        })
        .to_config()
        .expect("admissible");
        assert_eq!(config.keepalive, Some(Duration::from_secs(30)));
        assert_eq!(config.keepalive_interval, Some(Duration::from_secs(5)));
        assert_eq!(config.keepalive_retries, Some(3));

        let only_idle = TcpKeepalive::Probe(TcpProbe::new(30))
            .to_config()
            .expect("admissible");
        assert_eq!(only_idle.keepalive, Some(Duration::from_secs(30)));
        assert_eq!(only_idle.keepalive_interval, None);
        assert_eq!(only_idle.keepalive_retries, None);

        assert_eq!(
            TcpKeepalive::None.to_config().expect("none"),
            TcpConfig::default()
        );

        for (refused, key) in [
            (TcpProbe::new(0), "IdleSeconds"),
            (TcpProbe::new(-1), "IdleSeconds"),
            (TcpProbe::new(32768), "IdleSeconds"),
            (
                TcpProbe {
                    interval_seconds: Some(0),
                    ..TcpProbe::new(30)
                },
                "IntervalSeconds",
            ),
            (
                TcpProbe {
                    retries: Some(0),
                    ..TcpProbe::new(30)
                },
                "Retries",
            ),
            (
                TcpProbe {
                    retries: Some(128),
                    ..TcpProbe::new(30)
                },
                "Retries",
            ),
        ] {
            let error = TcpKeepalive::Probe(refused.clone())
                .to_config()
                .expect_err("out of its bounds");
            assert_eq!(error.key(), format!("Probe.{key}"), "{refused:?}");
        }
    }

    /// `None` is how a later source turns off what an earlier one set: an option left out leaves
    /// the earlier value, so "none" has to be a variant.
    #[test]
    fn a_none_turns_off_what_an_earlier_source_set_and_an_absent_option_leaves_it() {
        let earlier = ChannelOptions {
            transport: TransportOptions {
                tcp_keepalive: Some(TcpKeepalive::Probe(TcpProbe {
                    idle_seconds: 30,
                    interval_seconds: Some(5),
                    retries: Some(3),
                })),
                ..TransportOptions::default()
            },
            http2: Http2Options {
                keep_alive: Some(Http2KeepAlive::Ping(Http2Ping::new(Seconds(10.0)))),
                idle_timeout: Some(Http2IdleTimeout::After(Seconds(300.0))),
                simultaneous_calls_per_connection: Some(CallsPerConnection::Limit(4)),
                ..Http2Options::default()
            },
            ..ChannelOptions::default()
        };
        let nones = ChannelOptions {
            transport: TransportOptions {
                tcp_keepalive: Some(TcpKeepalive::None),
                ..TransportOptions::default()
            },
            http2: Http2Options {
                keep_alive: Some(Http2KeepAlive::None),
                idle_timeout: Some(Http2IdleTimeout::None),
                simultaneous_calls_per_connection: Some(CallsPerConnection::FromServer),
                ..Http2Options::default()
            },
            ..ChannelOptions::default()
        };

        let tcp_of = |options: &ChannelOptions| {
            options
                .transport
                .tcp_keepalive
                .as_ref()
                .expect("stated")
                .to_config()
                .expect("admissible")
        };
        let kept = ChannelOptions::default().over(&earlier);
        let tcp = tcp_of(&kept);
        assert_eq!(tcp.keepalive, Some(Duration::from_secs(30)));
        assert_eq!(tcp.keepalive_interval, Some(Duration::from_secs(5)));
        let http2 = kept.http2.to_config().expect("kept");
        assert_eq!(http2.keep_alive_interval, Some(Duration::from_secs(10)));
        assert_eq!(http2.idle_timeout, Some(Duration::from_secs(300)));
        assert_eq!(http2.simultaneous_calls_per_connection, Some(4));

        let off = nones.over(&earlier);
        let tcp = tcp_of(&off);
        assert_eq!(
            (tcp.keepalive, tcp.keepalive_interval, tcp.keepalive_retries),
            (None, None, None),
            "what an earlier source said of the probes is not read"
        );
        let http2 = off.http2.to_config().expect("off");
        assert_eq!(http2.keep_alive_interval, None);
        assert_eq!(http2.idle_timeout, None);
        assert_eq!(http2.simultaneous_calls_per_connection, None);
    }

    /// A value that is stated is checked against what its option admits.
    #[test]
    fn a_value_below_what_an_option_admits_is_refused() {
        for options in [
            Http2Options {
                keep_alive: Some(Http2KeepAlive::Ping(Http2Ping::new(Seconds(1e-10)))),
                ..Http2Options::default()
            },
            Http2Options {
                keep_alive: Some(Http2KeepAlive::Ping(Http2Ping::new(Seconds(0.0)))),
                ..Http2Options::default()
            },
            Http2Options {
                keep_alive: Some(Http2KeepAlive::Ping(Http2Ping {
                    timeout_seconds: Some(Seconds(0.0)),
                    ..Http2Ping::new(Seconds(1.0))
                })),
                ..Http2Options::default()
            },
            Http2Options {
                idle_timeout: Some(Http2IdleTimeout::After(Seconds(1e-10))),
                ..Http2Options::default()
            },
            Http2Options {
                idle_timeout: Some(Http2IdleTimeout::After(Seconds(0.0))),
                ..Http2Options::default()
            },
            Http2Options {
                simultaneous_calls_per_connection: Some(CallsPerConnection::Limit(0)),
                ..Http2Options::default()
            },
        ] {
            assert!(options.to_config().is_err(), "{options:?}");
        }
    }

    /// The keepalive and the idle timeout are alternatives, written and read as such, and a
    /// probe cannot be stated without its idle time.
    #[test]
    fn the_keepalives_are_read_as_alternatives_with_their_mandatory_fields() {
        let transport = |json: &str| serde_json::from_str::<TransportOptions>(json);
        assert_eq!(
            transport(r#"{"TcpKeepalive":"None"}"#)
                .expect("none")
                .tcp_keepalive,
            Some(TcpKeepalive::None)
        );
        assert_eq!(
            transport(r#"{"TcpKeepalive":{"Probe":{"IdleSeconds":30,"Retries":3}}}"#)
                .expect("a probe")
                .tcp_keepalive,
            Some(TcpKeepalive::Probe(TcpProbe {
                retries: Some(3),
                ..TcpProbe::new(30)
            }))
        );
        for refused in [
            r#"{"TcpKeepalive":{"Probe":{"IntervalSeconds":5}}}"#,
            r#"{"TcpKeepalive":{"Probe":{}}}"#,
            r#"{"TcpKeepalive":"Probe"}"#,
            r#"{"TcpKeepalive":{"None":true}}"#,
        ] {
            assert!(transport(refused).is_err(), "{refused}");
        }

        let http2 = |json: &str| serde_json::from_str::<Http2Options>(json);
        assert_eq!(
            http2(r#"{"KeepAlive":{"Ping":{"IntervalSeconds":10,"WhileIdle":true}}}"#)
                .expect("a ping")
                .keep_alive,
            Some(Http2KeepAlive::Ping(Http2Ping {
                while_idle: Some(true),
                ..Http2Ping::new(Seconds(10.0))
            }))
        );
        assert_eq!(
            http2(r#"{"IdleTimeout":{"After":300},"SimultaneousCallsPerConnection":{"Limit":1}}"#)
                .expect("a timeout and a limit"),
            Http2Options {
                idle_timeout: Some(Http2IdleTimeout::After(Seconds(300.0))),
                simultaneous_calls_per_connection: Some(CallsPerConnection::Limit(1)),
                ..Http2Options::default()
            }
        );
        assert_eq!(
            http2(r#"{"IdleTimeout":"None","SimultaneousCallsPerConnection":"FromServer"}"#)
                .expect("the neutral states"),
            Http2Options {
                idle_timeout: Some(Http2IdleTimeout::None),
                simultaneous_calls_per_connection: Some(CallsPerConnection::FromServer),
                ..Http2Options::default()
            }
        );
        for refused in [
            r#"{"KeepAlive":{"Ping":{"TimeoutSeconds":2}}}"#,
            r#"{"KeepAlive":"Ping"}"#,
            r#"{"IdleTimeout":"After"}"#,
            r#"{"SimultaneousCallsPerConnection":"Limit"}"#,
        ] {
            assert!(http2(refused).is_err(), "{refused}");
        }
    }

    #[test]
    fn an_initial_backoff_above_the_maximum_is_an_incoherence_naming_both_keys() {
        let options = RetryOptions::ExponentialBackoff(ExponentialBackoffOptions {
            initial_backoff_seconds: Some(Seconds(500.0)),
            ..ExponentialBackoffOptions::default()
        });

        let (config, incoherent) = options.convert().expect("each value is valid alone");
        let config = config.expect("a policy");
        assert_eq!(config.initial_backoff, Duration::from_secs(500));
        assert_eq!(
            config.max_backoff, config.initial_backoff,
            "the maximum is raised to the initial backoff"
        );
        let [incoherence] = incoherent.as_slice() else {
            panic!("{incoherent:?}");
        };
        assert!(incoherence.is_incoherence());
        assert_eq!(
            incoherence.keys().collect::<Vec<_>>(),
            [
                "ExponentialBackoff.InitialBackoffSeconds",
                "ExponentialBackoff.MaxBackoffSeconds"
            ]
        );

        let refused = options.to_config().expect_err("an incoherence is refused");
        assert!(refused.is_incoherence(), "{refused}");
    }

    #[test]
    fn the_retry_options_become_the_policy_and_one_that_cannot_back_off_is_refused() {
        let config = ExponentialBackoffOptions {
            max_attempts: Some(3),
            initial_backoff_seconds: Some(Seconds(0.5)),
            max_backoff_seconds: Some(Seconds(2.0)),
            backoff_multiplier: Some(2.0),
            failure_list: Some(vec![
                "Status.ABORTED".to_owned(),
                "Dial".to_owned(),
                "Status.ABORTED".to_owned(),
            ]),
        }
        .to_config()
        .expect("admissible");
        assert_eq!(config.max_attempts, 3);
        assert_eq!(config.initial_backoff, Duration::from_millis(500));
        assert_eq!(config.max_backoff, Duration::from_secs(2));
        assert_eq!(config.backoff_multiplier, 2.0);
        assert_eq!(
            config.failures,
            [Cause::Status(GrpcStatusCode::Aborted), Cause::Dial],
            "a list keeps its order and names each failure once"
        );
        assert_eq!(
            ExponentialBackoffOptions::default()
                .to_config()
                .expect("the defaults"),
            RetryConfig::default()
        );

        for (options, key) in [
            (
                ExponentialBackoffOptions {
                    max_attempts: Some(0),
                    ..ExponentialBackoffOptions::default()
                },
                "MaxAttempts",
            ),
            (
                ExponentialBackoffOptions {
                    initial_backoff_seconds: Some(Seconds(500.0)),
                    ..ExponentialBackoffOptions::default()
                },
                "InitialBackoffSeconds",
            ),
            (
                ExponentialBackoffOptions {
                    backoff_multiplier: Some(0.5),
                    ..ExponentialBackoffOptions::default()
                },
                "BackoffMultiplier",
            ),
            (
                ExponentialBackoffOptions {
                    backoff_multiplier: Some(f64::INFINITY),
                    ..ExponentialBackoffOptions::default()
                },
                "BackoffMultiplier",
            ),
        ] {
            let refused = options.to_config().expect_err(key);
            assert_eq!(refused.key(), key, "{refused}");
        }
    }

    /// An entry that names no failure is refused by its place in the list, and a list that is empty
    /// names nothing: it is no magic value.
    #[test]
    fn a_list_of_failures_names_each_entry_and_an_empty_one_names_nothing() {
        let retry = |entries: &[&str]| {
            ExponentialBackoffOptions {
                failure_list: Some(entries.iter().map(|entry| (*entry).to_owned()).collect()),
                ..ExponentialBackoffOptions::default()
            }
            .to_config()
        };

        assert_eq!(
            retry(&[
                "Status.UNAVAILABLE",
                "Http.503",
                "Reset.ENHANCE_YOUR_CALM",
                "Pushback",
                "Dial",
                "Connection",
            ])
            .expect("every kind")
            .failures,
            [
                Cause::Status(GrpcStatusCode::Unavailable),
                Cause::Http(503),
                Cause::Reset(11),
                Cause::Pushback,
                Cause::Dial,
                Cause::Connection,
            ]
        );
        assert_eq!(
            retry(&[]).expect("an empty list").failures,
            Vec::<Cause>::new(),
            "it retries nothing, and is not a refusal"
        );
        let refused = retry(&["Status.UNAVAILABLE", "Status.Unavailable"]).expect_err("a name");
        assert_eq!(refused.key(), "FailureList[1]");
        assert!(
            refused.to_string().contains("Status.Unavailable"),
            "{refused}"
        );
        for entry in ["", "OK", "Status.OK", "Http.99", "Reset.7", "Dial "] {
            assert!(retry(&[entry]).is_err(), "{entry:?}");
        }
        assert!(
            retry(&["Status.CANCELLED", "Status.DEADLINE_EXCEEDED"]).is_ok(),
            "a retry list names the statuses the specification does"
        );

        let throttle = |transient: &[&str], overload: &[&str]| {
            let list =
                |entries: &[&str]| Some(entries.iter().map(|entry| (*entry).to_owned()).collect());
            AdaptiveOptions {
                transient_list: list(transient),
                overload_list: list(overload),
                ..AdaptiveOptions::default()
            }
            .to_config()
        };
        let config = throttle(&["Dial"], &["Pushback", "Status.UNAVAILABLE"]).expect("lists");
        assert_eq!(config.transient, [Cause::Dial]);
        assert_eq!(
            config.overload,
            [Cause::Pushback, Cause::Status(GrpcStatusCode::Unavailable)]
        );
        let empty = throttle(&[], &[]).expect("empty lists");
        assert!(empty.transient.is_empty() && empty.overload.is_empty());
        for (transient, overload, key) in [
            (&["Status.CANCELLED"][..], &[][..], "TransientList[0]"),
            (
                &[][..],
                &["Dial", "Status.DEADLINE_EXCEEDED"][..],
                "OverloadList[1]",
            ),
            (&[][..], &["Http.1"][..], "OverloadList[0]"),
        ] {
            let refused = throttle(transient, overload).expect_err(key);
            assert_eq!(refused.key(), key, "{refused}");
        }
    }

    #[test]
    fn retry_is_exponential_backoff_unless_none_is_stated_and_none_retries_nothing() {
        let config = |retry: Option<RetryOptions>| retry.unwrap_or_default().to_config();

        assert_eq!(
            config(None).expect("the default"),
            Some(RetryConfig::default())
        );
        assert_eq!(
            config(Some(RetryOptions::ExponentialBackoff(
                ExponentialBackoffOptions {
                    max_attempts: Some(2),
                    ..ExponentialBackoffOptions::default()
                }
            )))
            .expect("two attempts")
            .map(|retry| retry.max_attempts),
            Some(2)
        );
        assert_eq!(
            config(Some(RetryOptions::None)).expect("none"),
            None,
            "None is no policy at all"
        );
    }

    /// A source that wants no retry states `None`: one attempt is not that.
    #[test]
    fn a_retry_that_retries_nothing_is_refused_unless_it_is_none() {
        for attempts in [0, 1] {
            let refused = RetryOptions::ExponentialBackoff(ExponentialBackoffOptions {
                max_attempts: Some(attempts),
                ..ExponentialBackoffOptions::default()
            })
            .to_config()
            .expect_err("too few");
            assert_eq!(refused.key(), "ExponentialBackoff.MaxAttempts");
            assert!(refused.to_string().contains("`None`"), "{refused}");
        }
    }

    #[test]
    fn the_outbound_traffic_options_are_read_and_merged_as_alternatives() {
        let read = |document: &str| serde_json::from_str::<GrpcOptions>(document);
        let traffic = |document: &str| read(document).map(|grpc| grpc.outbound_traffic);

        let none =
            traffic(r#"{"OutboundTraffic":{"Retry":"None","Throttle":"None"}}"#).expect("none");
        assert_eq!(none.retry, Some(RetryOptions::None));
        assert_eq!(none.throttle, Some(ThrottleOptions::None));
        let stated = traffic(
            r#"{"OutboundTraffic":{
                "Retry":{"ExponentialBackoff":{"MaxAttempts":3,"FailureList":["Dial"]}},
                "Throttle":{"Adaptive":{"FailureAllowance":4,"OverloadList":[]}},
                "Replay":{"MaxPerCallKiB":2,"MaxPerChannelKiB":8}}}"#,
        )
        .expect("stated");
        assert_eq!(
            stated.retry,
            Some(RetryOptions::ExponentialBackoff(
                ExponentialBackoffOptions {
                    max_attempts: Some(3),
                    failure_list: Some(vec!["Dial".to_owned()]),
                    ..ExponentialBackoffOptions::default()
                }
            ))
        );
        assert_eq!(
            stated.throttle,
            Some(ThrottleOptions::Adaptive(AdaptiveOptions {
                failure_allowance: Some(4),
                overload_list: Some(Vec::new()),
                ..AdaptiveOptions::default()
            }))
        );
        assert_eq!(stated.replay.max_per_call_kib, Some(2));
        assert_eq!(stated.replay.max_per_channel_kib, Some(8));
        assert_eq!(
            traffic("{}").expect("nothing"),
            OutboundTrafficOptions::default()
        );
        for document in [
            r#"{"OutboundTraffic":{"Retry":{"None":null,"ExponentialBackoff":{}}}}"#,
            r#"{"OutboundTraffic":{"Retry":{"None":false}}}"#,
            r#"{"OutboundTraffic":{"Throttle":{"None":null,"Adaptive":{}}}}"#,
        ] {
            assert!(read(document).is_err(), "{document}");
        }

        let over = |own: RetryOptions, default: RetryOptions| {
            let with = |retry| ChannelOptions {
                grpc: GrpcOptions {
                    outbound_traffic: OutboundTrafficOptions {
                        retry: Some(retry),
                        ..OutboundTrafficOptions::default()
                    },
                    ..GrpcOptions::default()
                },
                ..ChannelOptions::default()
            };
            with(own).over(&with(default)).grpc.outbound_traffic.retry
        };
        let attempts = |max_attempts| {
            RetryOptions::ExponentialBackoff(ExponentialBackoffOptions {
                max_attempts,
                ..ExponentialBackoffOptions::default()
            })
        };
        assert_eq!(
            over(RetryOptions::None, attempts(Some(4))),
            Some(RetryOptions::None),
            "None over ExponentialBackoff replaces it whole"
        );
        assert_eq!(
            over(attempts(Some(3)), RetryOptions::None),
            Some(attempts(Some(3))),
            "and ExponentialBackoff over None takes nothing of it"
        );
        let merged = over(
            RetryOptions::ExponentialBackoff(ExponentialBackoffOptions {
                failure_list: Some(vec!["Connection".to_owned()]),
                ..ExponentialBackoffOptions::default()
            }),
            RetryOptions::ExponentialBackoff(ExponentialBackoffOptions {
                max_attempts: Some(4),
                failure_list: Some(vec!["Dial".to_owned(), "Pushback".to_owned()]),
                ..ExponentialBackoffOptions::default()
            }),
        );
        let Some(RetryOptions::ExponentialBackoff(merged)) = merged else {
            panic!("{merged:?}");
        };
        assert_eq!(merged.max_attempts, Some(4), "field by field");
        assert_eq!(
            merged.failure_list,
            Some(vec!["Connection".to_owned()]),
            "and a list is one value, stated whole"
        );
    }

    #[test]
    fn the_replay_options_are_kibibytes_and_default_to_the_engines() {
        assert_eq!(
            ReplayOptions::default().to_config().expect("the defaults"),
            ReplayConfig::default()
        );
        let config = ReplayOptions {
            max_per_call_kib: Some(2),
            max_per_channel_kib: Some(0),
        }
        .to_config()
        .expect("stated");
        assert_eq!((config.call_bytes, config.channel_bytes), (2048, 0));
        let refused = ReplayOptions {
            max_per_channel_kib: Some(-1),
            ..ReplayOptions::default()
        }
        .to_config()
        .expect_err("negative");
        assert_eq!(refused.key(), "MaxPerChannelKiB");
        assert_eq!(
            serde_json::to_string(&ReplayOptions {
                max_per_call_kib: Some(1),
                max_per_channel_kib: Some(2),
            })
            .expect("written"),
            r#"{"MaxPerCallKiB":1,"MaxPerChannelKiB":2}"#
        );
    }

    #[test]
    fn the_throttle_is_adaptive_unless_none_is_stated() {
        let config = |options: Option<ThrottleOptions>| options.unwrap_or_default().to_config();

        assert_eq!(
            config(None).expect("the default"),
            Some(AdaptiveConfig::default())
        );
        assert_eq!(config(Some(ThrottleOptions::None)).expect("none"), None);

        let stated = config(Some(ThrottleOptions::Adaptive(AdaptiveOptions {
            multiplier: Some(3.0),
            throttle_multiplier: Some(1.5),
            failure_allowance: Some(7),
            window_seconds: Some(Seconds(10.0)),
            floor_per_second: Some(2.0),
            ..AdaptiveOptions::default()
        })))
        .expect("stated")
        .expect("a judgment");
        assert_eq!(stated.multiplier, 3.0);
        assert_eq!(stated.throttle_multiplier, 1.5);
        assert_eq!(stated.slack, 7);
        assert_eq!(stated.window, Duration::from_secs(10));
        assert_eq!(stated.floor_per_second, 2.0);
        assert_eq!(stated.transient, default_lists().0);
        assert_eq!(stated.overload, default_lists().1);
    }

    fn default_lists() -> (Vec<Cause>, Vec<Cause>) {
        let defaults = AdaptiveConfig::default();
        (defaults.transient, defaults.overload)
    }

    #[test]
    fn each_bound_of_the_throttle_is_refused_naming_its_key() {
        for (options, key) in [
            (
                AdaptiveOptions {
                    multiplier: Some(0.9),
                    ..AdaptiveOptions::default()
                },
                "Multiplier",
            ),
            (
                AdaptiveOptions {
                    multiplier: Some(101.0),
                    ..AdaptiveOptions::default()
                },
                "Multiplier",
            ),
            (
                AdaptiveOptions {
                    multiplier: Some(f64::NAN),
                    ..AdaptiveOptions::default()
                },
                "Multiplier",
            ),
            (
                AdaptiveOptions {
                    throttle_multiplier: Some(0.0),
                    ..AdaptiveOptions::default()
                },
                "ThrottleMultiplier",
            ),
            (
                AdaptiveOptions {
                    failure_allowance: Some(-1),
                    ..AdaptiveOptions::default()
                },
                "FailureAllowance",
            ),
            (
                AdaptiveOptions {
                    failure_allowance: Some(1_000_001),
                    ..AdaptiveOptions::default()
                },
                "FailureAllowance",
            ),
            (
                AdaptiveOptions {
                    window_seconds: Some(Seconds(0.011)),
                    ..AdaptiveOptions::default()
                },
                "WindowSeconds",
            ),
            (
                AdaptiveOptions {
                    window_seconds: Some(Seconds(601.0)),
                    ..AdaptiveOptions::default()
                },
                "WindowSeconds",
            ),
            (
                AdaptiveOptions {
                    floor_per_second: Some(0.0),
                    ..AdaptiveOptions::default()
                },
                "FloorPerSecond",
            ),
            (
                AdaptiveOptions {
                    floor_per_second: Some(f64::INFINITY),
                    ..AdaptiveOptions::default()
                },
                "FloorPerSecond",
            ),
        ] {
            let refused = ThrottleOptions::Adaptive(options)
                .to_config()
                .expect_err(key);
            assert_eq!(refused.key(), format!("Adaptive.{key}"), "{refused}");
        }
    }

    #[test]
    fn the_throttle_is_read_and_merged_as_an_alternative() {
        let read = |document: &str| serde_json::from_str::<OutboundTrafficOptions>(document);

        assert_eq!(
            read(r#"{"Throttle":{"Adaptive":{"Multiplier":3,"FailureAllowance":0}}}"#)
                .expect("adaptive")
                .throttle,
            Some(ThrottleOptions::Adaptive(AdaptiveOptions {
                multiplier: Some(3.0),
                failure_allowance: Some(0),
                ..AdaptiveOptions::default()
            }))
        );
        assert!(
            read(r#"{"Throttle":{"Elsewhere":true}}"#).is_err(),
            "a key that names none of the variants is refused"
        );

        let over = |own: ThrottleOptions, default: ThrottleOptions| {
            let with = |throttle| ChannelOptions {
                grpc: GrpcOptions {
                    outbound_traffic: OutboundTrafficOptions {
                        throttle: Some(throttle),
                        ..OutboundTrafficOptions::default()
                    },
                    ..GrpcOptions::default()
                },
                ..ChannelOptions::default()
            };
            with(own)
                .over(&with(default))
                .grpc
                .outbound_traffic
                .throttle
        };
        let adaptive = |multiplier, failure_allowance| {
            ThrottleOptions::Adaptive(AdaptiveOptions {
                multiplier,
                failure_allowance,
                ..AdaptiveOptions::default()
            })
        };
        assert_eq!(
            over(ThrottleOptions::None, adaptive(Some(3.0), None)),
            Some(ThrottleOptions::None),
            "None over Adaptive replaces it whole"
        );
        assert_eq!(
            over(adaptive(Some(4.0), None), ThrottleOptions::None),
            Some(adaptive(Some(4.0), None))
        );
        assert_eq!(
            over(adaptive(None, Some(0)), adaptive(Some(3.0), None)),
            Some(adaptive(Some(3.0), Some(0))),
            "an Adaptive over an Adaptive merges field by field"
        );
    }

    #[test]
    fn the_http2_options_become_the_session_configuration() {
        let config = Http2Options {
            keep_alive: Some(Http2KeepAlive::Ping(Http2Ping {
                timeout_seconds: Some(Seconds(2.5)),
                while_idle: Some(true),
                ..Http2Ping::new(Seconds(10.0))
            })),
            idle_timeout: Some(Http2IdleTimeout::After(Seconds(300.0))),
            simultaneous_calls_per_connection: Some(CallsPerConnection::Limit(1)),
            send: Http2SendOptions {
                coalescing_bytes: Some(0),
                stream_buffer_kib: Some(4),
                frames_per_write: Some(1),
                header_list_bytes: Some(HeaderListBytes::Max(8192)),
            },
            receive: Some(Http2ReceiveOptions::Fixed(Http2FixedWindows {
                stream_window_bytes: Some(1024),
                connection_window_bytes: Some(65_535),
            })),
        }
        .to_config()
        .expect("admissible");
        assert_eq!(config.idle_timeout, Some(Duration::from_secs(300)));
        assert_eq!(config.simultaneous_calls_per_connection, Some(1));
        assert_eq!(config.write_coalescing, 0);
        assert_eq!(config.send_buffer, 4096, "KiB are counted in bytes");
        assert_eq!(config.max_header_list_size, Some(8192));
        assert_eq!(config.keep_alive_interval, Some(Duration::from_secs(10)));
        assert_eq!(config.keep_alive_timeout, Duration::from_millis(2500));
        assert!(config.keep_alive_while_idle);
        assert_eq!(
            config.receive_windows,
            ReceiveWindows::Fixed(FixedWindows {
                stream: 1024,
                connection: 65_535
            })
        );

        assert_eq!(
            Http2Options::default().to_config().expect("the defaults"),
            Http2Config::default()
        );

        let refused = Http2Options {
            receive: Some(Http2ReceiveOptions::Fixed(Http2FixedWindows {
                connection_window_bytes: Some(65_534),
                ..Http2FixedWindows::default()
            })),
            ..Http2Options::default()
        }
        .to_config()
        .expect_err("below the window every connection starts with");
        assert_eq!(refused.key(), "Receive.Fixed.ConnectionWindowBytes");

        let refused = Http2Options {
            send: Http2SendOptions {
                coalescing_bytes: Some(-1),
                ..Http2SendOptions::default()
            },
            ..Http2Options::default()
        }
        .to_config()
        .expect_err("a negative size");
        assert_eq!(refused.key(), "Send.CoalescingBytes");

        let refused = Http2Options {
            send: Http2SendOptions {
                stream_buffer_kib: Some(0),
                ..Http2SendOptions::default()
            },
            ..Http2Options::default()
        }
        .to_config()
        .expect_err("a buffer that never takes a byte");
        assert_eq!(refused.key(), "Send.StreamBufferKiB");

        let refused = Http2Options {
            send: Http2SendOptions {
                stream_buffer_kib: Some(LARGEST_STREAM_BUFFER_KIB + 1),
                ..Http2SendOptions::default()
            },
            ..Http2Options::default()
        }
        .to_config()
        .expect_err("a buffer the session cannot count");
        assert_eq!(refused.key(), "Send.StreamBufferKiB");
        let largest = Http2Options {
            send: Http2SendOptions {
                stream_buffer_kib: Some(LARGEST_STREAM_BUFFER_KIB),
                ..Http2SendOptions::default()
            },
            ..Http2Options::default()
        }
        .to_config()
        .expect("the largest buffer");
        assert_eq!(largest.send_buffer, u32::MAX as usize - 1023);

        let refused = Http2Options {
            simultaneous_calls_per_connection: Some(CallsPerConnection::Limit(0)),
            ..Http2Options::default()
        }
        .to_config()
        .expect_err("a connection that carries no call");
        assert_eq!(refused.key(), "SimultaneousCallsPerConnection.Limit");

        let refused = Http2Options {
            send: Http2SendOptions {
                header_list_bytes: Some(HeaderListBytes::Max(0)),
                ..Http2SendOptions::default()
            },
            ..Http2Options::default()
        }
        .to_config()
        .expect_err("a header list no request fits");
        assert_eq!(refused.key(), "Send.HeaderListBytes.Max");

        let unbounded = Http2Options {
            send: Http2SendOptions {
                header_list_bytes: Some(HeaderListBytes::Unbounded),
                ..Http2SendOptions::default()
            },
            ..Http2Options::default()
        }
        .to_config()
        .expect("no bound");
        assert_eq!(unbounded.max_header_list_size, None);

        for (frames, why) in [(0, "between 1 and"), (257, "between 1 and")] {
            let refused = Http2Options {
                send: Http2SendOptions {
                    frames_per_write: Some(frames),
                    ..Http2SendOptions::default()
                },
                ..Http2Options::default()
            }
            .to_config()
            .expect_err("frames per write out of range");
            assert_eq!(refused.key(), "Send.FramesPerWrite");
            assert!(refused.to_string().contains(why), "{refused}");
        }
        let sixteen = Http2Options {
            send: Http2SendOptions {
                frames_per_write: Some(16),
                ..Http2SendOptions::default()
            },
            ..Http2Options::default()
        }
        .to_config();
        if cfg!(h2_batch) {
            assert_eq!(sixteen.expect("the patched build").frames_per_write, 16);
        } else {
            let refused = sixteen.expect_err("a build without the patch");
            assert!(refused.to_string().contains("h2-batch"), "{refused}");
        }
    }

    /// An option stated over a default wins, one left out is the default's, and a struct merges
    /// the same way within it.
    #[test]
    fn a_stated_option_wins_over_its_default() {
        let credits = |credits| GrpcOptions {
            host: HostOptions {
                receive: HostReceiveOptions {
                    window: Some(credits),
                    ..HostReceiveOptions::default()
                },
                ..HostOptions::default()
            },
            ..GrpcOptions::default()
        };
        let defaults = ChannelOptions {
            grpc: GrpcOptions {
                user_agent: Some("default".to_owned()),
                ..credits(2)
            },
            http2: Http2Options {
                keep_alive: Some(Http2KeepAlive::Ping(Http2Ping {
                    while_idle: Some(true),
                    ..Http2Ping::new(Seconds(10.0))
                })),
                receive: Some(Http2ReceiveOptions::Fixed(Http2FixedWindows {
                    stream_window_bytes: Some(70_000),
                    ..Http2FixedWindows::default()
                })),
                ..Http2Options::default()
            },
            ..ChannelOptions::default()
        };
        let merged = ChannelOptions {
            grpc: credits(3),
            http2: Http2Options {
                keep_alive: Some(Http2KeepAlive::Ping(Http2Ping::new(Seconds(5.0)))),
                receive: Some(Http2ReceiveOptions::Fixed(Http2FixedWindows {
                    stream_window_bytes: Some(80_000),
                    ..Http2FixedWindows::default()
                })),
                ..Http2Options::default()
            },
            ..ChannelOptions::default()
        }
        .over(&defaults);

        assert_eq!(merged.grpc.user_agent.as_deref(), Some("default"));
        assert_eq!(merged.grpc.host.receive.window, Some(3));
        assert_eq!(
            merged.http2.keep_alive,
            Some(Http2KeepAlive::Ping(Http2Ping::new(Seconds(5.0)))),
            "a ping, which has a mandatory interval, replaces the default's whole"
        );
        assert_eq!(
            merged.http2.receive,
            Some(Http2ReceiveOptions::Fixed(Http2FixedWindows {
                stream_window_bytes: Some(80_000),
                ..Http2FixedWindows::default()
            }))
        );
    }

    /// A size in KiB, a bound in bytes and a deadline are read under the names the schema spells,
    /// the acronym's capital included, and a state is a variant: a key spelled otherwise is
    /// refused, and the value never taken.
    #[test]
    fn the_sizes_and_the_deadline_are_read_under_their_names_and_merged_as_variants() {
        let options: ChannelOptions = crate::configuration::Configuration::with_prefix("")
            .document(
                r#"{"Grpc":{"Deadline":{"Default":2.5},
                    "Send":{"MessageSizeKiB":{"Max":2},"Compression":"Zstd"},
                    "Receive":{"MessageSizeKiB":"Unbounded"}},
                   "Http2":{"Send":{"StreamBufferKiB":3,"HeaderListBytes":{"Max":8192}}}}"#,
            )
            .load()
            .expect("a document");
        assert_eq!(options.grpc.deadline, Some(Deadline::Default(Seconds(2.5))));
        assert_eq!(
            options.grpc.send.message_size_kib,
            Some(SendMessageSizeKiB::Max(2))
        );
        assert_eq!(options.grpc.send.compression, Some(SendCompression::Zstd));
        assert_eq!(
            options.grpc.receive.message_size_kib,
            Some(ReceiveMessageSizeKiB::Unbounded)
        );
        assert_eq!(options.http2.send.stream_buffer_kib, Some(3));
        assert_eq!(
            options.http2.send.header_list_bytes,
            Some(HeaderListBytes::Max(8192))
        );

        // A later source takes the other variant whole, or the default's where it states none.
        let none = ChannelOptions {
            grpc: GrpcOptions {
                deadline: Some(Deadline::None),
                send: GrpcSendOptions {
                    message_size_kib: Some(SendMessageSizeKiB::Unbounded),
                    compression: Some(SendCompression::None),
                },
                ..GrpcOptions::default()
            },
            ..ChannelOptions::default()
        };
        let over = none.over(&options);
        assert_eq!(over.grpc.deadline, Some(Deadline::None));
        assert_eq!(
            over.grpc.send.message_size_kib,
            Some(SendMessageSizeKiB::Unbounded)
        );
        assert_eq!(over.grpc.send.compression, Some(SendCompression::None));
        assert_eq!(
            over.grpc.receive.message_size_kib,
            Some(ReceiveMessageSizeKiB::Unbounded)
        );
        assert_eq!(over.http2.send.stream_buffer_kib, Some(3));

        // A size past what an address holds is the largest one.
        assert_eq!(
            SendMessageSizeKiB::Max(i32::MAX).limit().expect("a limit"),
            Some((i32::MAX as usize).saturating_mul(1024))
        );
        assert_eq!(
            ReceiveMessageSizeKiB::Unbounded.limit().expect("a limit"),
            usize::MAX
        );
        for refused in [
            r#"{"Deadline":{"Default":"2"}}"#,
            r#"{"Deadline":"Default"}"#,
            r#"{"Deadline":{"None":true}}"#,
            r#"{"Send":{"MessageSizeKiB":"Max"}}"#,
            r#"{"Send":{"Compression":{"Gzip":true}}}"#,
        ] {
            assert!(
                serde_json::from_str::<GrpcOptions>(refused).is_err(),
                "{refused}"
            );
        }
    }

    /// The memory ceiling is a pair in MiB: read under the schema's spelling, merged field by
    /// field, converted to bytes, and checked once merged, a hard ceiling below the soft one being
    /// refused naming both keys.
    #[test]
    fn the_memory_ceiling_is_a_pair_of_mib_checked_once_merged() {
        let read = |json: &str| {
            crate::configuration::Configuration::with_prefix("")
                .document(json)
                .load::<RuntimeOptions>()
                .expect("a document")
        };
        let soft = read(r#"{"MemoryCeiling":{"SoftMiB":8}}"#);
        let hard = read(r#"{"MemoryCeiling":{"HardMiB":4}}"#);
        assert_eq!(soft.memory_ceiling.soft_mib, Some(8));
        assert_eq!(hard.memory_ceiling.hard_mib, Some(4));
        assert_eq!(soft.memory_ceiling.bytes(), (8 << 20, 0));
        assert_eq!(hard.memory_ceiling.bytes(), (0, 4 << 20));
        assert_eq!(
            MemoryCeilingOptions::default().bytes(),
            (0, 0),
            "the library's own"
        );
        soft.memory_ceiling.check().expect("a soft ceiling alone");
        assert_eq!(
            hard.memory_ceiling
                .check()
                .expect_err("under the default")
                .keys()
                .collect::<Vec<_>>(),
            ["MemoryCeiling.SoftMiB", "MemoryCeiling.HardMiB"],
            "a hard ceiling is held against the soft one's default"
        );

        let merged = crate::configuration::Document::over(hard.clone(), soft.clone());
        assert_eq!(merged.memory_ceiling.soft_mib, Some(8));
        assert_eq!(merged.memory_ceiling.hard_mib, Some(4));
        let refused = merged.memory_ceiling.check().expect_err("hard below soft");
        assert!(refused.is_incoherence(), "{refused}");
        assert!(
            refused
                .to_string()
                .starts_with("MemoryCeiling.SoftMiB and MemoryCeiling.HardMiB are incoherent"),
            "{refused}"
        );

        for (zero, key) in [
            (
                r#"{"MemoryCeiling":{"SoftMiB":0}}"#,
                "MemoryCeiling.SoftMiB",
            ),
            (
                r#"{"MemoryCeiling":{"HardMiB":0}}"#,
                "MemoryCeiling.HardMiB",
            ),
        ] {
            let refused = crate::configuration::Configuration::with_prefix("")
                .document(zero)
                .load::<RuntimeOptions>()
                .expect_err("a ceiling of zero is out of the schema's bounds");
            assert_eq!(refused.key(), Some(key), "{refused}");
            assert!(
                refused.to_string().contains("it has to be between 1 and"),
                "{refused}"
            );
        }
        let number = crate::configuration::Configuration::with_prefix("")
            .document(r#"{"MemoryCeiling":1048576}"#)
            .load::<RuntimeOptions>();
        assert!(
            number.is_err(),
            "a number where a pair is expected is refused"
        );
        let unknown = crate::configuration::Configuration::with_prefix("")
            .document(r#"{"MemoryHardCeiling":1048576}"#)
            .load::<RuntimeOptions>()
            .expect_err("a key the schema does not know is refused");
        assert_eq!(unknown.key(), Some("MemoryHardCeiling"));
    }

    /// The two directions are stated apart, an encoding that is not named is refused, and a
    /// direction left out is the default's.
    #[test]
    fn compression_is_stated_per_direction() {
        let read = |json: &str| serde_json::from_str::<ChannelOptions>(json);

        let sending = read(r#"{"Grpc":{"Send":{"Compression":"Gzip"}}}"#).expect("gzip is named");
        assert_eq!(sending.grpc.send.compression, Some(SendCompression::Gzip));
        assert_eq!(sending.grpc.receive.compression, None);

        for (name, encoding, wire) in [
            (
                "Gzip",
                SendCompression::Gzip,
                Some(crate::grpc::Encoding::Gzip),
            ),
            (
                "Deflate",
                SendCompression::Deflate,
                Some(crate::grpc::Encoding::Deflate),
            ),
            (
                "Zstd",
                SendCompression::Zstd,
                Some(crate::grpc::Encoding::Zstd),
            ),
            ("None", SendCompression::None, None),
        ] {
            let sends = read(&format!(
                r#"{{"Grpc":{{"Send":{{"Compression":"{name}"}}}}}}"#
            ))
            .expect("a declared name");
            assert_eq!(sends.grpc.send.compression, Some(encoding));
            assert_eq!(encoding.encoding(), wire);
        }

        for refused in [
            r#"{"Grpc":{"Send":{"Compression":"Brotli"}}}"#,
            r#"{"Grpc":{"Send":{"Compression":"gzip"}}}"#,
            r#"{"Grpc":{"Send":{"Compression":["Gzip"]}}}"#,
            r#"{"Grpc":{"Receive":{"Compression":"Gzip"}}}"#,
            r#"{"Grpc":{"Receive":{"Compression":["gzip"]}}}"#,
            r#"{"Grpc":{"Receive":{"Compression":["None"]}}}"#,
            r#"{"Grpc":{"Receive":{"Compression":["Gzip","Brotli"]}}}"#,
        ] {
            assert!(read(refused).is_err(), "{refused}");
        }
    }

    /// What a channel accepts is a list: kept in the order stated, a repeat included (the channel
    /// collapses it), and stated whole over the defaults', an empty list included.
    #[test]
    fn the_accepted_encodings_are_a_list_stated_whole() {
        use MessageEncoding::*;
        let read = |json: &str| serde_json::from_str::<ChannelOptions>(json).expect("options");
        let accepts = |options: &ChannelOptions| options.grpc.receive.compression.clone();

        let stated = read(r#"{"Grpc":{"Receive":{"Compression":["Zstd","Gzip","Zstd"]}}}"#);
        assert_eq!(accepts(&stated), Some(vec![Zstd, Gzip, Zstd]));
        assert_eq!(
            serde_json::to_value(&stated).expect("a document")["Grpc"]["Receive"],
            serde_json::json!({"Compression": ["Zstd", "Gzip", "Zstd"]}),
            "written as an array of names, in order"
        );

        let defaults = read(r#"{"Grpc":{"Receive":{"Compression":["Gzip","Deflate"]}}}"#);
        assert_eq!(
            accepts(&stated.clone().over(&defaults)),
            Some(vec![Zstd, Gzip, Zstd]),
            "the list stated replaces the default's, it is not appended to it"
        );
        assert_eq!(
            accepts(&ChannelOptions::default().over(&defaults)),
            Some(vec![Gzip, Deflate]),
            "a channel that states none has the default's"
        );
        let empty = read(r#"{"Grpc":{"Receive":{"Compression":[]}}}"#);
        assert_eq!(
            accepts(&empty.over(&defaults)),
            Some(vec![]),
            "an empty list is stated, and clears the default's"
        );
    }

    /// The adaptive windows are an alternative to the fixed ones: either, stated over the other,
    /// replaces it, and a session that states neither keeps the default's.
    #[test]
    fn adaptive_windows_are_an_alternative_to_fixed_ones() {
        let adaptive = Http2Options {
            receive: Some(Http2ReceiveOptions::Adaptive),
            ..Http2Options::default()
        };
        let fixed = Http2Options {
            receive: Some(Http2ReceiveOptions::Fixed(Http2FixedWindows {
                stream_window_bytes: Some(70_000),
                ..Http2FixedWindows::default()
            })),
            ..Http2Options::default()
        };
        let unstated = Http2Options {
            idle_timeout: Some(Http2IdleTimeout::None),
            ..Http2Options::default()
        };

        assert_eq!(
            adaptive.to_config().expect("admissible").receive_windows,
            ReceiveWindows::Adaptive
        );
        assert_eq!(adaptive.clone().over(&fixed).receive, adaptive.receive);
        assert_eq!(fixed.clone().over(&adaptive).receive, fixed.receive);
        assert_eq!(unstated.clone().over(&adaptive).receive, adaptive.receive);
        assert_eq!(
            unstated.to_config().expect("admissible").receive_windows,
            ReceiveWindows::default()
        );
    }

    /// A later source returns to the system's roots and to no client certificate over a root and
    /// a certificate an earlier one named: the neutral states are variants, which an absent key
    /// could not say.
    #[test]
    fn the_system_roots_and_no_certificate_replace_what_an_earlier_source_set() {
        let earlier: TlsOptions = serde_json::from_str(
            r#"{"ServerCertificates":{"CaPem":"ca.pem"},"ClientCertificate":{"P12":{"Path":"me.p12"}}}"#,
        )
        .expect("an earlier source");
        let later: TlsOptions =
            serde_json::from_str(r#"{"ServerCertificates":"System","ClientCertificate":"None"}"#)
                .expect("a later source");
        assert_eq!(later.server_certificates, Some(ServerCertificates::System));
        assert_eq!(later.client_certificate, Some(ClientCertificate::None));

        let merged = later.over(&earlier);
        assert_eq!(merged.server_certificates, Some(ServerCertificates::System));
        assert_eq!(merged.client_certificate, Some(ClientCertificate::None));
        let config = merged.load().expect("nothing to read");
        assert!(config.roots.is_empty());
        assert!(!config.accept_any_server);
        assert!(config.identity.is_none());

        // And what a source leaves out is what the earlier one set.
        let kept = TlsOptions::default().over(&earlier);
        assert_eq!(
            kept.server_certificates,
            Some(ServerCertificates::CaPem("ca.pem".to_owned()))
        );
    }

    /// An alternative stated over another is taken whole: nothing of the default's is combined
    /// into it. Beside it, every other option cumulates, the two backoff bounds included.
    #[test]
    fn a_stated_alternative_replaces_the_default_whole() {
        let mut url = ProxyUrl::new("http://proxy.test:3128");
        url.username = Some("someone".to_owned());
        let defaults = ChannelOptions {
            transport: TransportOptions {
                tls: TlsOptions {
                    server_certificates: Some(ServerCertificates::CaPem("ca.pem".to_owned())),
                    client_certificate: Some(ClientCertificate::P12(P12Certificate::new(
                        "me.p12", None,
                    ))),
                },
                proxy: Some(ProxyOptions::Url(url)),
                ..TransportOptions::default()
            },
            grpc: GrpcOptions {
                outbound_traffic: OutboundTrafficOptions {
                    retry: Some(RetryOptions::ExponentialBackoff(
                        ExponentialBackoffOptions {
                            max_backoff_seconds: Some(Seconds(5.0)),
                            ..ExponentialBackoffOptions::default()
                        },
                    )),
                    ..OutboundTrafficOptions::default()
                },
                ..GrpcOptions::default()
            },
            ..ChannelOptions::default()
        };
        let merged = ChannelOptions {
            transport: TransportOptions {
                tls: TlsOptions {
                    server_certificates: Some(ServerCertificates::None),
                    ..TlsOptions::default()
                },
                proxy: Some(ProxyOptions::None),
                ..TransportOptions::default()
            },
            grpc: GrpcOptions {
                outbound_traffic: OutboundTrafficOptions {
                    retry: Some(RetryOptions::ExponentialBackoff(
                        ExponentialBackoffOptions {
                            initial_backoff_seconds: Some(Seconds(10.0)),
                            ..ExponentialBackoffOptions::default()
                        },
                    )),
                    ..OutboundTrafficOptions::default()
                },
                ..GrpcOptions::default()
            },
            ..ChannelOptions::default()
        }
        .over(&defaults);

        let tls = &merged.transport.tls;
        assert_eq!(tls.server_certificates, Some(ServerCertificates::None));
        assert_eq!(
            tls.client_certificate,
            Some(ClientCertificate::P12(P12Certificate::new("me.p12", None))),
            "the identity is another alternative, which the channel leaves to its default"
        );
        assert_eq!(merged.transport.proxy, Some(ProxyOptions::None));
        let Some(RetryOptions::ExponentialBackoff(retry)) = &merged.grpc.outbound_traffic.retry
        else {
            panic!("{:?}", merged.grpc.outbound_traffic.retry);
        };
        assert_eq!(retry.initial_backoff_seconds, Some(Seconds(10.0)));
        assert_eq!(retry.max_backoff_seconds, Some(Seconds(5.0)));
    }

    /// An alternative stated over the same one merges as its payload does: a payload with a
    /// mandatory field, which each one stated here has, replaces the default's whole, an optional
    /// field it leaves out included.
    #[test]
    fn a_stated_alternative_over_the_same_one_replaces_a_payload_with_a_mandatory_field() {
        let mut default_url = ProxyUrl::new("http://default.test:3128");
        default_url.username = Some("alice".to_owned());
        default_url.password = Some(Password::new("s3cret"));
        let mut store = StoreCertificate::new(StoreSearch::FriendlyName("root".to_owned()));
        store.location = Some(StoreLocation::LocalMachine);
        let defaults = ChannelOptions {
            transport: TransportOptions {
                tls: TlsOptions {
                    server_certificates: Some(ServerCertificates::CaStore(store)),
                    client_certificate: Some(ClientCertificate::P12(P12Certificate::new(
                        "default.p12",
                        Some(Password::new("bundle")),
                    ))),
                    ..TlsOptions::default()
                },
                proxy: Some(ProxyOptions::Url(default_url)),
                ..TransportOptions::default()
            },
            ..ChannelOptions::default()
        };
        let mut own_url = ProxyUrl::new("http://own.test:3128");
        own_url.username = Some("bob".to_owned());
        let mut own_store = StoreCertificate::new(StoreSearch::Thumbprint("ab".to_owned()));
        own_store.name = Some("Pinned".to_owned());
        let merged = ChannelOptions {
            transport: TransportOptions {
                tls: TlsOptions {
                    server_certificates: Some(ServerCertificates::CaStore(own_store)),
                    client_certificate: Some(ClientCertificate::P12(P12Certificate::new(
                        "own.p12", None,
                    ))),
                    ..TlsOptions::default()
                },
                proxy: Some(ProxyOptions::Url(own_url)),
                ..TransportOptions::default()
            },
            ..ChannelOptions::default()
        }
        .over(&defaults);

        let Some(ProxyOptions::Url(url)) = &merged.transport.proxy else {
            panic!("{:?}", merged.transport.proxy);
        };
        assert_eq!(url.address, "http://own.test:3128");
        assert_eq!(url.username.as_deref(), Some("bob"));
        assert_eq!(url.password, None, "the default's password is not paired");
        let Some(ServerCertificates::CaStore(store)) = &merged.transport.tls.server_certificates
        else {
            panic!("{:?}", merged.transport.tls.server_certificates);
        };
        assert_eq!(store.find, StoreSearch::Thumbprint("ab".to_owned()));
        assert_eq!(store.name.as_deref(), Some("Pinned"));
        assert_eq!(
            store.location, None,
            "the default's location is not combined with the store stated"
        );
        assert_eq!(
            merged.transport.tls.client_certificate,
            Some(ClientCertificate::P12(P12Certificate::new("own.p12", None))),
            "the default's password is not paired"
        );
    }

    /// A proxy URL and a bundle have a mandatory field, so each is stated whole: nothing of the
    /// default's credentials is taken, the same address or path included.
    #[test]
    fn a_proxy_url_and_a_bundle_are_stated_whole() {
        let mut default_url = ProxyUrl::new("http://proxy.test:3128");
        default_url.username = Some("alice".to_owned());
        default_url.password = Some(Password::new("s3cret"));
        let defaults = ChannelOptions {
            transport: TransportOptions {
                tls: TlsOptions {
                    client_certificate: Some(ClientCertificate::P12(P12Certificate::new(
                        "me.p12",
                        Some(Password::new("bundle")),
                    ))),
                    ..TlsOptions::default()
                },
                proxy: Some(ProxyOptions::Url(default_url)),
                ..TransportOptions::default()
            },
            ..ChannelOptions::default()
        };
        let merged_with = |username: Option<&str>| {
            let mut own_url = ProxyUrl::new("http://proxy.test:3128");
            own_url.username = username.map(str::to_owned);
            ChannelOptions {
                transport: TransportOptions {
                    tls: TlsOptions {
                        client_certificate: Some(ClientCertificate::P12(P12Certificate::new(
                            "me.p12", None,
                        ))),
                        ..TlsOptions::default()
                    },
                    proxy: Some(ProxyOptions::Url(own_url)),
                    ..TransportOptions::default()
                },
                ..ChannelOptions::default()
            }
            .over(&defaults)
        };

        let merged = merged_with(None);
        let Some(ProxyOptions::Url(url)) = &merged.transport.proxy else {
            panic!("{:?}", merged.transport.proxy);
        };
        assert_eq!(url.username, None);
        assert_eq!(url.password, None);

        let Some(ProxyOptions::Url(url)) = merged_with(Some("bob")).transport.proxy else {
            panic!("a Url is merged into a Url");
        };
        assert_eq!(url.username.as_deref(), Some("bob"));
        assert_eq!(url.password, None);
        assert_eq!(
            merged.transport.tls.client_certificate,
            Some(ClientCertificate::P12(P12Certificate::new("me.p12", None)))
        );
    }

    /// A group with a mandatory field is stated whole, over the same variant or not, and one with
    /// only optional fields merges field by field.
    #[test]
    fn a_group_with_a_mandatory_field_is_stated_whole() {
        let probe = |own: TcpProbe| TransportOptions {
            tcp_keepalive: Some(TcpKeepalive::Probe(own)),
            ..TransportOptions::default()
        };
        let earlier = probe(TcpProbe {
            interval_seconds: Some(10),
            retries: Some(3),
            ..TcpProbe::new(60)
        });
        assert_eq!(
            probe(TcpProbe::new(30)).over(&earlier),
            probe(TcpProbe::new(30)),
            "the interval and the count of the earlier probe are not kept"
        );
        assert_eq!(
            TransportOptions {
                tcp_keepalive: Some(TcpKeepalive::None),
                ..TransportOptions::default()
            }
            .over(&earlier)
            .tcp_keepalive,
            Some(TcpKeepalive::None),
            "another variant is taken whole"
        );
        assert_eq!(
            TransportOptions::default().over(&earlier),
            earlier,
            "a probe left out is the earlier one"
        );

        let windows = |stream: Option<i32>, connection: Option<i32>| Http2Options {
            receive: Some(Http2ReceiveOptions::Fixed(Http2FixedWindows {
                stream_window_bytes: stream,
                connection_window_bytes: connection,
            })),
            ..Http2Options::default()
        };
        assert_eq!(
            windows(Some(80_000), None).over(&windows(Some(70_000), Some(90_000))),
            windows(Some(80_000), Some(90_000)),
            "windows have no mandatory field"
        );
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

    /// The runtime's schema is what generates its C# class, kept as the channel's is.
    #[cfg(feature = "schema")]
    #[test]
    fn the_committed_runtime_schema_is_the_one_the_types_describe() {
        let committed = include_str!("../runtime.schema.json").replace("\r\n", "\n");

        assert_eq!(
            committed,
            runtime_schema(),
            "the options changed and the schema did not; write it again with\n  \
             cargo run -p armonik-transport --features schema --example runtime_schema -- \
             packages/rust/armonik-transport/runtime.schema.json"
        );
    }

    /// Every option the schema declares is one `serde` reads, under the name the schema spells,
    /// and so is every alternative of each `oneOf`. A document holds one alternative per choice,
    /// so one is written per index, and the index picks the alternatives of nested choices digit by
    /// digit: a choice has at most three, and nests at most two deep, so nine indices reach every
    /// one.
    ///
    /// The two derives are separate readings of the same fields, and this crate makes them differ
    /// on purpose - `schemars(with = "i32")` states a schema the field's own type would not. A
    /// name they stopped agreeing on would be an option the generated C# sets and the schema
    /// admits, which the loader would refuse by its path: what this asserts is that the document
    /// is read.
    #[cfg(feature = "schema")]
    #[test]
    fn every_option_the_schema_declares_is_one_serde_reads() {
        fn read_all<D: crate::configuration::Document + std::fmt::Debug>(rendered: &str) {
            let schema: serde_json::Value =
                serde_json::from_str(rendered).expect("the schema is a document");

            for alternative in 0..9 {
                let document = a_value_for(&schema, &schema, alternative);
                let read = crate::configuration::Configuration::with_prefix("")
                    .document(document.to_string())
                    .load::<D>();

                assert!(
                    read.is_ok(),
                    "the schema declares {document}, which the loader refuses: {}",
                    read.unwrap_err()
                );
            }
        }

        read_all::<ChannelOptions>(&schema());
        read_all::<RuntimeOptions>(&runtime_schema());
    }

    /// A value each property of `node` admits, as one document naming all of them. A `oneOf`
    /// takes the alternative `alternative` picks in the base of its alternatives' count, and hands
    /// what is left of the index to the choices that alternative holds.
    ///
    /// Values rather than a name list, so that the document is read as well as named.
    #[cfg(feature = "schema")]
    fn a_value_for(
        node: &serde_json::Value,
        root: &serde_json::Value,
        alternative: usize,
    ) -> serde_json::Value {
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

        if let Some(constant) = node.get("const") {
            return constant.clone();
        }
        if let Some(alternatives) = node.get("oneOf").and_then(Value::as_array) {
            let picked = &alternatives[alternative % alternatives.len()];
            return a_value_for(picked, root, alternative / alternatives.len());
        }

        match node.get("type").and_then(Value::as_str) {
            Some("object") | None => {
                let properties = node
                    .get("properties")
                    .and_then(Value::as_object)
                    .expect("an object states its properties");

                Value::Object(
                    properties
                        .iter()
                        .map(|(name, property)| {
                            (name.clone(), a_value_for(property, root, alternative))
                        })
                        .collect(),
                )
            }
            // A value every bound in this schema admits: an integer's own `minimum` where it
            // states one, which is below its `maximum`, and a number of 1, which the nanosecond
            // and the multipliers' bounds admit.
            Some("integer") => json!(node.get("minimum").and_then(Value::as_i64).unwrap_or(1)),
            Some("number") => json!(1.0),
            Some("string") => json!("x"),
            Some("boolean") => json!(true),
            // One item, of the type the schema states for them.
            Some("array") => json!([a_value_for(&node["items"], root, alternative)]),
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
