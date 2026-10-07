//! The options a caller sets on a channel, as a document carries them.
//!
//! Structured and typed, because the schema derived from these types is what generates the
//! options class a .NET caller fills in: a number is a number, a group of options is an object,
//! and every constraint that can be said here is said here rather than only in the code that
//! enforces it. What a type cannot say - that an endpoint names a scheme this engine speaks -
//! the transport says, by option name.
//!
//! A key no type declares is read past rather than refused, so that the `configuration` loader logs
//! it and goes on; the schema still states `additionalProperties: false`, which tells whoever edits
//! a document what the engine will log. An alternative - how the server is verified, who the client
//! is, which proxy - reads a key that names none of its variants the same way, as no alternative.

use std::time::Duration;

use hyper::Uri;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use secrecy::ExposeSecret;

use crate::grpc::RetryConfig;
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
/// The endpoint is not here: a channel is opened on its own, or on the Endpoint of its runtime's
/// options. Every option has a default, so naming none of them is a valid configuration.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "PascalCase"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
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
    /// Defaults to `{"System": {}}`: the proxy the system names, if any, with no credentials of
    /// its own.
    #[cfg_attr(
        feature = "serde",
        serde(
            default,
            deserialize_with = "alternative::optional",
            skip_serializing_if = "Option::is_none"
        )
    )]
    #[cfg_attr(feature = "schema", schemars(with = "ProxyOptions"))]
    pub proxy: Option<ProxyOptions>,

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

/// `true`, the value of an alternative that carries nothing: a key names an alternative, and this
/// is what it is set to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Chosen;

#[cfg(feature = "serde")]
impl serde::Serialize for Chosen {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bool(true)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Chosen {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if bool::deserialize(deserializer)? {
            Ok(Chosen)
        } else {
            Err(serde::de::Error::custom(
                "an alternative is chosen with `true`; one not chosen is left out",
            ))
        }
    }
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for Chosen {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Chosen".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "const": true })
    }
}

/// An HTTP proxy, which a dial tunnels through with `CONNECT`, so TLS stays end to end with the
/// server.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum ProxyOptions {
    /// No proxy: every dial goes to the endpoint itself.
    None(Chosen),

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
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "PascalCase"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct ProxyCredentials {
    /// The username, which `Basic` forbids a `:` in.
    ///
    /// Ignored when the system names no proxy. Beside the environment's proxy, it takes the place
    /// of the username that proxy's URL carries; beside the one Windows' settings name, it is the
    /// username. Taken from the runtime's channel defaults, with their `Password`, only when these
    /// options state neither.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub username: Option<String>,

    /// The password that goes with `Username`.
    ///
    /// Ignored when the system names no proxy. Beside the environment's proxy, it takes the place
    /// of the password that proxy's URL carries; beside the one Windows' settings name, it is the
    /// password. Taken from the runtime's channel defaults, with their `Username`, only when these
    /// options state neither.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing))]
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub password: Option<Password>,
}

/// A proxy named by its address.
#[derive(Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "PascalCase"))]
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
    ///
    /// Taken from the runtime's channel defaults, with their `Password`, only when they name the
    /// same `Address` and these options state neither.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub username: Option<String>,

    /// The password that goes with `Username`.
    ///
    /// Taken from the runtime's channel defaults, with their `Username`, only when they name the
    /// same `Address` and these options state neither.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing))]
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub password: Option<Password>,
}

/// A proxy's `http://` URL that carries its credentials, as `user:password@`, percent-encoded;
/// `http://` is assumed when no scheme is written.
#[derive(Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
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
            Self::None(Chosen) => Ok(ProxyConfig::default()),
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
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "PascalCase"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct TlsOptions {
    /// How the server certificate is verified.
    ///
    /// Defaults to the system's roots.
    #[cfg_attr(
        feature = "serde",
        serde(
            default,
            deserialize_with = "alternative::optional",
            skip_serializing_if = "Option::is_none"
        )
    )]
    #[cfg_attr(feature = "schema", schemars(with = "ServerVerification"))]
    pub server: Option<ServerVerification>,

    /// The certificate the client presents, and its key.
    ///
    /// Defaults to none.
    #[cfg_attr(
        feature = "serde",
        serde(
            default,
            deserialize_with = "alternative::optional",
            skip_serializing_if = "Option::is_none"
        )
    )]
    #[cfg_attr(feature = "schema", schemars(with = "ClientCertificate"))]
    pub client: Option<ClientCertificate>,

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

/// How the server certificate is verified.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum ServerVerification {
    /// Against the roots of a PEM file, named by its path, in place of the system's. Every
    /// certificate the file holds is a root.
    CaPem(#[cfg_attr(feature = "schema", schemars(length(min = 1)))] String),

    /// Against a root from a Windows certificate store, `Root` unless `Name` says otherwise, in
    /// place of the system's.
    ///
    /// Refused off Windows.
    CaStore(StoreCertificate),

    /// Not at all: any server certificate is accepted. The connection is still encrypted, to
    /// whoever answers.
    Unverified(Chosen),
}

/// The certificate the client presents, and its key.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum ClientCertificate {
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
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "PascalCase"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct PemCertificate {
    /// Path to a PEM file of the client's certificate, then each issuer the server may not hold.
    #[cfg_attr(feature = "schema", schemars(length(min = 1)))]
    pub certificate: String,

    /// Path to a PEM file of the certificate's key.
    #[cfg_attr(feature = "schema", schemars(length(min = 1)))]
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
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "PascalCase"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct P12Certificate {
    /// Path to a PKCS#12 bundle of the client's certificate, the issuers it carries and the key.
    #[cfg_attr(feature = "schema", schemars(length(min = 1)))]
    pub path: String,

    /// The password the bundle is protected by.
    ///
    /// Defaults to the empty one. Taken from the runtime's channel defaults only when they name
    /// the same `Path`.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing))]
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
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "PascalCase"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct StoreCertificate {
    /// Where the store is.
    ///
    /// Defaults to `CurrentUser`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "StoreLocation"))]
    pub location: Option<StoreLocation>,

    /// The store's name, such as `My`, `Root` or `CA`. Defaults to the one its option states.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "String", length(min = 1)))]
    pub name: Option<String>,

    /// How the certificate is found in the store.
    #[cfg_attr(feature = "serde", serde(deserialize_with = "alternative::required"))]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum StoreLocation {
    /// The current user's stores.
    CurrentUser,

    /// The machine's stores, which every user shares.
    LocalMachine,
}

/// How a certificate is found in its store.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum StoreSearch {
    /// By its SHA-1 fingerprint, as 40 hexadecimal digits; spaces and colons between them are
    /// ignored.
    Thumbprint(#[cfg_attr(feature = "schema", schemars(length(min = 1)))] String),

    /// By a text its subject contains, compared without case, as .NET's `FindBySubjectName`
    /// compares it.
    SubjectName(#[cfg_attr(feature = "schema", schemars(length(min = 1)))] String),

    /// By its friendly name, exactly.
    FriendlyName(#[cfg_attr(feature = "schema", schemars(length(min = 1)))] String),
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
#[cfg_attr(feature = "serde", serde(rename_all = "PascalCase"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
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
#[cfg_attr(feature = "serde", serde(rename_all = "PascalCase"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
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

    /// How long a connection stays open with no call on it before it is closed, the next call
    /// dialling a new one. Each connection has its own. A call holds its connection to the end of
    /// its response and of its request.
    ///
    /// Defaults to none: an idle connection stays open.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Seconds", extend("minimum" = 1e-9))
    )]
    pub idle_timeout_seconds: Option<Seconds>,

    /// How many calls one connection carries at once, never more than its server allows. A call
    /// that finds every connection full opens another, as many as the calls in flight need, and
    /// each closes on its own idle timeout when IdleTimeoutSeconds is set. At 1, calls follow one
    /// another on a connection but never share it, so that a GOAWAY a server sends because of one
    /// call - nginx's ENHANCE_YOUR_CALM against too many resets, for one - ends that call alone.
    ///
    /// Defaults to none: a connection carries as many calls as its server allows.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "i32", range(min = 1)))]
    pub simultaneous_calls_per_connection: Option<i32>,

    /// What the session sends.
    ///
    /// Defaults to `{}`, which leaves each of its options at its own default.
    #[cfg_attr(feature = "serde", serde(default))]
    pub send: Http2SendOptions,

    /// What the session lets the peer send.
    ///
    /// Defaults to `{"Fixed": {}}`: windows of 2 MiB per call and 5 MiB for the connection.
    #[cfg_attr(
        feature = "serde",
        serde(
            default,
            deserialize_with = "alternative::optional",
            skip_serializing_if = "Option::is_none"
        )
    )]
    #[cfg_attr(feature = "schema", schemars(with = "Http2ReceiveOptions"))]
    pub receive: Option<Http2ReceiveOptions>,
}

/// What an HTTP/2 session sends.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "PascalCase"))]
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
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "i32", range(min = 0)))]
    pub coalescing_bytes: Option<i32>,

    /// How many bytes of one call's request may be queued in the session, waiting to be written,
    /// before its next part is handed over. A part is handed over whole once fewer bytes than this
    /// are queued, and the peer's window has room, so up to one part more than this is queued.
    ///
    /// Defaults to 1048576, 1 MiB.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "i32", range(min = 1)))]
    pub stream_buffer_size: Option<i32>,

    /// How many DATA frames of the peer's largest size one queued part of a request may span,
    /// written one after the other in one write: a large message then goes out in fewer, larger
    /// writes. Above 1, a call reset while its part is being written can still send up to this
    /// many frames less one before its reset, and a PING or a SETTINGS acknowledgement queued
    /// behind DATA waits for this many times more of it. Above 1 needs an engine built against
    /// the h2-batch patch (`packages/rust/patches/h2-batch`), and is refused otherwise. At most
    /// 256.
    ///
    /// Defaults to 1.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "i32", range(min = 1, max = LARGEST_FRAMES_PER_WRITE))
    )]
    pub frames_per_write: Option<i32>,
}

/// What an HTTP/2 session lets its peer send ahead of what is read: windows of fixed sizes, or
/// windows that grow with what the link carries.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum Http2ReceiveOptions {
    /// Windows of fixed sizes, announced as the session opens.
    Fixed(Http2FixedWindows),

    /// Windows that grow with the link: both start at 65535, the size every connection starts
    /// with, and grow with the bandwidth-delay product the session's PINGs measure, up to 16 MiB.
    /// Neither shrinks.
    Adaptive(Chosen),
}

/// HTTP/2 flow-control windows of fixed sizes.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "PascalCase"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct Http2FixedWindows {
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
}

/// When a failed call is sent again, as gRFC A6 has it: after a backoff drawn below a bound
/// that starts at `InitialBackoffSeconds` and grows by `BackoffMultiplier` to
/// `MaxBackoffSeconds`, for UNAVAILABLE, ABORTED and UNKNOWN, while no response head has reached
/// the reader and what the call sent is still kept for the replay.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "PascalCase"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
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
        let (roots, accept_any_server) = match &self.server {
            None => (Vec::new(), false),
            Some(ServerVerification::CaPem(path)) => (certificates("Server.CaPem", path)?, false),
            Some(ServerVerification::CaStore(store)) => (
                vec![store
                    .root()
                    .map_err(|refused| refused.under("Server.CaStore"))?],
                false,
            ),
            Some(ServerVerification::Unverified(Chosen)) => (Vec::new(), true),
        };

        let identity = match &self.client {
            None => None,
            Some(ClientCertificate::Pem(pem)) => {
                Some(pem.load().map_err(|refused| refused.under("Client.Pem"))?)
            }
            Some(ClientCertificate::P12(p12)) => Some(
                pkcs12("Path", &p12.path, p12.password.as_ref())
                    .map_err(|refused| refused.under("Client.P12"))?,
            ),
            Some(ClientCertificate::Store(store)) => Some(
                store
                    .identity()
                    .map_err(|refused| refused.under("Client.Store"))?,
            ),
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
            receive_windows: match &self.receive {
                None => defaults.receive_windows,
                Some(Http2ReceiveOptions::Fixed(windows)) => {
                    let fixed = FixedWindows::default();
                    ReceiveWindows::Fixed(FixedWindows {
                        stream: window(
                            "Receive.Fixed.StreamWindowSize",
                            windows.stream_window_size,
                            1,
                            fixed.stream,
                        )?,
                        connection: window(
                            "Receive.Fixed.ConnectionWindowSize",
                            windows.connection_window_size,
                            65_535,
                            fixed.connection,
                        )?,
                    })
                }
                Some(Http2ReceiveOptions::Adaptive(Chosen)) => ReceiveWindows::Adaptive,
            },
            idle_timeout: duration("IdleTimeoutSeconds", self.idle_timeout_seconds, 1e-9, None)?,
            simultaneous_calls_per_connection: match self.simultaneous_calls_per_connection {
                None => defaults.simultaneous_calls_per_connection,
                Some(calls) if calls < 1 => {
                    return Err(OptionRefusal::new(
                        "SimultaneousCallsPerConnection",
                        format!("{calls} has to be at least 1"),
                    ))
                }
                Some(calls) => Some(calls as usize),
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
            send_buffer: match self.send.stream_buffer_size {
                None => defaults.send_buffer,
                Some(size) if size < 1 => {
                    return Err(OptionRefusal::new(
                        "Send.StreamBufferSize",
                        format!("{size} has to be at least 1"),
                    ))
                }
                Some(size) => size as usize,
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
        })
    }
}

/// What a caller may set on one channel.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "PascalCase"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct ChannelOptions {
    /// What the transport does, beyond reaching the endpoint.
    ///
    /// Defaults to `{}`, which leaves each of its options at its own default.
    #[cfg_attr(feature = "serde", serde(default))]
    pub transport: TransportOptions,

    /// The HTTP/2 session the channel's calls share.
    ///
    /// Defaults to `{}`, which leaves each of its options at its own default.
    #[cfg_attr(feature = "serde", serde(default))]
    pub http2: Http2Options,

    /// What the channel's calls do: their messages, deadlines and retries, and what crosses
    /// between the host and the engine.
    ///
    /// Defaults to `{}`, which leaves each of its options at its own default.
    #[cfg_attr(feature = "serde", serde(default))]
    pub grpc: GrpcOptions,
}

/// What the channel's calls do: their messages, deadlines and retries, and what crosses between
/// the host and the engine.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "PascalCase"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct GrpcOptions {
    /// What this client calls itself in `user-agent`.
    ///
    /// Defaults to `armonik-transport/` followed by the engine's version.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "String", length(min = 1)))]
    pub user_agent: Option<String>,

    /// The deadline of a call that states none, counted from its start: the call ends
    /// `DEADLINE_EXCEEDED` once it passes, and the server is told what was left of it when the
    /// call started as `grpc-timeout`. It bounds the whole call, a streaming one included, and not
    /// only the wait for the response's head. A call's own deadline takes its place, and a call
    /// that states none takes this one.
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

    /// When a failed call is sent again.
    ///
    /// Defaults to `{}`: five attempts in all, as `GrpcClient` has them. A call its peer never
    /// processed goes again besides, whatever `MaxAttempts` is, while every message it sent is
    /// kept.
    #[cfg_attr(feature = "serde", serde(default))]
    pub retry: RetryOptions,

    /// What a call sends to the server.
    ///
    /// Defaults to `{}`, which leaves each of its options at its own default.
    #[cfg_attr(feature = "serde", serde(default))]
    pub send: GrpcSendOptions,

    /// What a call accepts from the server.
    ///
    /// Defaults to `{}`, which leaves each of its options at its own default.
    #[cfg_attr(feature = "serde", serde(default))]
    pub receive: GrpcReceiveOptions,

    /// What crosses between the host and the engine on each call.
    ///
    /// Defaults to `{}`, which leaves each of its options at its own default.
    #[cfg_attr(feature = "serde", serde(default))]
    pub host: HostOptions,
}

/// What a call sends to the server.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "PascalCase"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct GrpcSendOptions {
    /// The largest message this client will send, in bytes. A larger one ends its call
    /// `RESOURCE_EXHAUSTED`, and none of it is sent.
    ///
    /// Defaults to none, any message a call is given going out. Zero is refused: it admits only
    /// empty messages.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "i32", range(min = 1)))]
    pub max_message_size: Option<i32>,
}

/// What a call accepts from the server.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "PascalCase"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct GrpcReceiveOptions {
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
    pub max_message_size: Option<i32>,
}

/// What crosses between the host and the engine on each call, one way and the other.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "PascalCase"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct HostOptions {
    /// What the host sends.
    ///
    /// Defaults to `{}`, which leaves each of its options at its own default.
    #[cfg_attr(feature = "serde", serde(default))]
    pub send: HostSendOptions,

    /// What the engine delivers to the host.
    ///
    /// Defaults to `{}`, which leaves each of its options at its own default.
    #[cfg_attr(feature = "serde", serde(default))]
    pub receive: HostReceiveOptions,
}

/// What a call's host sends: the messages it hands the engine.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "PascalCase"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct HostSendOptions {
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
    pub window: Option<i32>,
}

/// What the engine delivers to a call's host: its payloads and its status.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "PascalCase"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct HostReceiveOptions {
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
    pub window: Option<i32>,

    /// How many bytes of a response a delivery to the host may wait to gather, so that a unary
    /// answer's head, message and status reach it in one callback. 0 delivers each read at once.
    ///
    /// Defaults to 16384, 16 KiB.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "i32", range(min = 0)))]
    pub coalescing_bytes: Option<i32>,
}

/// Options stated over their defaults: a struct field by field, recursively, and an option is the
/// default's where it is not stated. An alternative stated over the same one merges its fields
/// the same way; over another, it is taken whole, so two alternatives are never combined into one
/// neither stated.
trait Over {
    fn over(self, defaults: &Self) -> Self;
}

impl<T: Over + Clone> Over for Option<T> {
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
    Chosen,
    StoreLocation,
    CredentialedUrl,
);

/// `Over` for an enum whose every variant carries one value: the same variant merges what the two
/// carry, and another is taken whole. Every variant is listed and matched without `_`, so a
/// variant the enum gains and this does not list fails to compile - and so is the list of names
/// [`alternative`] reads a key against.
macro_rules! over_variants {
    ($type:ident { $($variant:ident),+ $(,)? }) => {
        impl Over for $type {
            fn over(self, defaults: &Self) -> Self {
                match self {
                    $(Self::$variant(own) => Self::$variant(match defaults {
                        Self::$variant(default) => own.over(default),
                        _ => own,
                    }),)+
                }
            }
        }

        #[cfg(feature = "serde")]
        impl alternative::Alternative for $type {
            const NAME: &'static str = stringify!($type);
            const VARIANTS: &'static [&'static str] = &[$(stringify!($variant)),+];
        }
    };
}

/// How an alternative is read: an object whose one key names a variant and holds what it carries.
///
/// By hand rather than by serde's derive, which refuses a key that names no variant. Such a key is
/// read past instead, as a struct reads past a key it does not declare, so that the configuration
/// loader logs it; the alternative is then none, and keeps what an earlier source gave it.
#[cfg(feature = "serde")]
mod alternative {
    use std::marker::PhantomData;

    use serde::de::{
        self, DeserializeOwned, DeserializeSeed, Deserializer, EnumAccess, IgnoredAny,
        IntoDeserializer, MapAccess, VariantAccess, Visitor,
    };

    /// An enum read as an alternative, by the names of its variants.
    pub(super) trait Alternative: DeserializeOwned {
        const NAME: &'static str;
        const VARIANTS: &'static [&'static str];
    }

    /// An alternative that may be left out, and is none when its key names no variant.
    pub(super) fn optional<'de, D: Deserializer<'de>, T: Alternative>(
        deserializer: D,
    ) -> Result<Option<T>, D::Error> {
        deserializer.deserialize_option(Optional(PhantomData))
    }

    /// An alternative a document has to state, refused when its key names no variant.
    pub(super) fn required<'de, D: Deserializer<'de>, T: Alternative>(
        deserializer: D,
    ) -> Result<T, D::Error> {
        deserializer
            .deserialize_struct(T::NAME, T::VARIANTS, Chosen(PhantomData))?
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
            // As a struct whose fields are the variants, so that a reader matching keys to fields
            // matches these too.
            deserializer.deserialize_struct(T::NAME, T::VARIANTS, Chosen(PhantomData))
        }
    }

    /// The variant an object's keys name, if one does; two are refused.
    struct Chosen<T>(PhantomData<T>);

    impl<'de, T: Alternative> Visitor<'de> for Chosen<T> {
        type Value = Option<T>;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "an object naming one of {}", T::VARIANTS.join(", "))
        }

        fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Option<T>, M::Error> {
            let mut chosen = None;
            while let Some(key) = map.next_key::<String>()? {
                if !T::VARIANTS.contains(&key.as_str()) {
                    map.next_value::<IgnoredAny>()?;
                    continue;
                }
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

over_variants!(ServerVerification {
    CaPem,
    CaStore,
    Unverified,
});
over_variants!(ClientCertificate { Pem, P12, Store });
over_variants!(ProxyOptions {
    None,
    System,
    Url,
    UrlWithCredentials,
});
over_variants!(StoreSearch {
    Thumbprint,
    SubjectName,
    FriendlyName,
});

/// `Over` for a struct of options, every field merged. The fields are destructured without `..`,
/// so a field the struct gains and this does not list fails to compile.
macro_rules! over_fields {
    ($type:ident { $($field:ident),+ $(,)? }) => {
        impl Over for $type {
            fn over(self, defaults: &Self) -> Self {
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
over_fields!(GrpcOptions {
    user_agent,
    default_deadline_seconds,
    retry,
    send,
    receive,
    host,
});
over_fields!(GrpcSendOptions { max_message_size });
over_fields!(GrpcReceiveOptions { max_message_size });
over_fields!(HostOptions { send, receive });
over_fields!(HostSendOptions { window });
over_fields!(HostReceiveOptions {
    window,
    coalescing_bytes,
});
over_fields!(TlsOptions {
    server,
    client,
    override_target_name,
});
over_fields!(TcpKeepaliveOptions {
    idle_seconds,
    interval_seconds,
    retries,
});
over_fields!(Http2Options {
    keep_alive_interval_seconds,
    keep_alive_timeout_seconds,
    keep_alive_while_idle,
    idle_timeout_seconds,
    simultaneous_calls_per_connection,
    send,
    receive,
});
over_fields!(Http2SendOptions {
    coalescing_bytes,
    stream_buffer_size,
    frames_per_write,
});
over_fields!(Http2FixedWindows {
    stream_window_size,
    connection_window_size,
});
over_variants!(Http2ReceiveOptions { Fixed, Adaptive });
over_fields!(RetryOptions {
    max_attempts,
    initial_backoff_seconds,
    max_backoff_seconds,
    backoff_multiplier,
    call_replay_bytes,
    channel_replay_bytes,
});
over_fields!(PemCertificate { certificate, key });

/// A username and its password are one credential: stating either states it, and nothing of the
/// default's is paired with it.
impl Over for ProxyCredentials {
    fn over(self, defaults: &Self) -> Self {
        let Self { username, password } = &self;
        if username.is_some() || password.is_some() {
            self
        } else {
            defaults.clone()
        }
    }
}

/// Credentials go with the proxy they were stated for: the default's are taken only for the same
/// address, and whole, as `ProxyCredentials` takes them; another address is the channel's own,
/// with its own credentials or none.
impl Over for ProxyUrl {
    fn over(self, defaults: &Self) -> Self {
        let Self {
            address,
            username,
            password,
        } = self;
        let stated = username.is_some() || password.is_some();
        if address != defaults.address || stated {
            return Self {
                address,
                username,
                password,
            };
        }
        Self {
            address,
            username: defaults.username.clone(),
            password: defaults.password.clone(),
        }
    }
}

/// A password goes with the bundle it opens: the default's is taken only for the same path.
impl Over for P12Certificate {
    fn over(self, defaults: &Self) -> Self {
        if self.path != defaults.path {
            return self;
        }
        let Self { path, password } = self;
        Self {
            password: password.over(&defaults.password),
            path,
        }
    }
}
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
    /// and one left out the default's. An alternative - how the server is verified, who the client
    /// is, which proxy - stated over the same one merges its fields the same way, and over another
    /// is taken whole. Options that only bound one another, such as the two backoff bounds, merge
    /// as any option, and a merge where they disagree is refused as a document stating both would
    /// be.
    pub fn over(self, defaults: &Self) -> Self {
        Over::over(self, defaults)
    }
}

/// What a caller may set on the runtime: the endpoint, the memory ceilings, the options every
/// channel takes where its own state none, and what the engine logs.
#[derive(Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "PascalCase"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[non_exhaustive]
pub struct RuntimeOptions {
    /// The server, as `http://host:port` in the clear or `https://host:port` over TLS, that a
    /// channel created with no endpoint of its own reaches.
    ///
    /// Defaults to none: every channel then names its own.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "String", length(min = 1)))]
    pub endpoint: Option<String>,

    /// The bytes the runtime holds before work waits, counting the buffers lent to send and the
    /// messages received until the host gives them back: a call stops reading, and a send waits
    /// for room.
    ///
    /// Defaults to 4294967295, four gigabytes, or half the address space where that is smaller;
    /// a larger value is that too.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "i64", range(min = 1)))]
    pub memory_ceiling: Option<u64>,

    /// The bytes past which the runtime stops: a received message that would take the count past
    /// them ends its call with RESOURCE_EXHAUSTED. Calls admitted to read below MemoryCeiling may
    /// pass it together, by a message each, and this bounds them. At least MemoryCeiling, or
    /// MemoryCeiling's default when that is left out.
    ///
    /// Defaults to a quarter above MemoryCeiling.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "i64", range(min = 1)))]
    pub memory_hard_ceiling: Option<u64>,

    /// Channel options every channel of the runtime takes where its own options state none: the
    /// two are merged option by option, a struct's options within it, and the channel's win; an
    /// alternative - how the server is verified, who the client is, which proxy - merges its fields
    /// over the same alternative and is taken whole over another.
    ///
    /// Defaults to none.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "ChannelOptions"))]
    pub channel_defaults: Option<ChannelOptions>,

    /// What the engine reports of itself to the host.
    ///
    /// Defaults to `{}`, which leaves each of its options at its own default.
    #[cfg_attr(feature = "serde", serde(default))]
    pub logging: LoggingOptions,
}

/// Which of the engine's log events a host receives.
///
/// Ignored by a Rust host.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "PascalCase"))]
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
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub filter: Option<String>,
}

/// The endpoint is printed elided, since it may carry a password.
impl std::fmt::Debug for RuntimeOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeOptions")
            .field("endpoint", &self.endpoint.as_deref().map(elided))
            .field("memory_ceiling", &self.memory_ceiling)
            .field("memory_hard_ceiling", &self.memory_hard_ceiling)
            .field("channel_defaults", &self.channel_defaults)
            .field("logging", &self.logging)
            .finish()
    }
}

over_fields!(RuntimeOptions {
    endpoint,
    memory_ceiling,
    memory_hard_ceiling,
    channel_defaults,
    logging,
});
over_fields!(LoggingOptions { filter });

#[cfg(feature = "configuration")]
impl crate::configuration::Document for ChannelOptions {
    fn over(self, earlier: Self) -> Self {
        Over::over(self, &earlier)
    }
}

#[cfg(feature = "configuration")]
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
            server: Some(ServerVerification::CaPem(write(
                &directory,
                "ca.pem",
                &certificate,
            ))),
            client: Some(ClientCertificate::Pem(PemCertificate::new(
                write(&directory, "chain.pem", &two),
                write(&directory, "key.pem", &key),
            ))),
            override_target_name: Some("server.test".to_owned()),
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

        let unverified = TlsOptions {
            server: Some(ServerVerification::Unverified(Chosen)),
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
            client: Some(ClientCertificate::Pem(PemCertificate::new(
                certificate,
                key,
            ))),
            ..TlsOptions::default()
        };

        for (options, key) in [
            (
                TlsOptions {
                    server: Some(ServerVerification::CaPem(missing.clone())),
                    ..TlsOptions::default()
                },
                "Server.CaPem",
            ),
            (
                TlsOptions {
                    server: Some(ServerVerification::CaPem(empty.clone())),
                    ..TlsOptions::default()
                },
                "Server.CaPem",
            ),
            (pem(&missing, &certificate), "Client.Pem.Certificate"),
            (pem(&certificate, &certificate), "Client.Pem.Key"),
            (pem(&certificate, &missing), "Client.Pem.Key"),
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

    /// Alternatives exclude one another by their shape: a document naming two is refused as it
    /// is read, before any file is.
    #[cfg(feature = "serde")]
    #[test]
    fn a_document_naming_two_alternatives_is_refused() {
        for document in [
            r#"{"Server":{"CaPem":"ca.pem","Unverified":true}}"#,
            r#"{"Client":{"Pem":{"Certificate":"c.pem","Key":"k.pem"},"P12":{"Path":"c.p12"}}}"#,
            r#"{"Server":{"Unverified":false}}"#,
            r#"{"Client":{"Pem":{"Certificate":"c.pem"}}}"#,
            r#"{"Client":{"P12":{"Password":"s3cret"}}}"#,
        ] {
            let read = serde_json::from_str::<TlsOptions>(document);
            assert!(read.is_err(), "{document}");
            assert!(
                !read.unwrap_err().to_string().contains("s3cret"),
                "{document}"
            );
        }
        let read: TlsOptions = serde_json::from_str(
            r#"{"Server":{"Unverified":true},"Client":{"P12":{"Path":"c.p12","Password":"x"}}}"#,
        )
        .expect("one alternative each");
        assert_eq!(read.server, Some(ServerVerification::Unverified(Chosen)));
        assert_eq!(
            read.client,
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
            client: Some(ClientCertificate::P12(P12Certificate::new(
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
            assert_eq!(refused.key(), "Client.P12.Path", "{refused}");
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
                    client: Some(ClientCertificate::Store(store.clone())),
                    ..TlsOptions::default()
                },
                "Client.Store",
            ),
            (
                TlsOptions {
                    server: Some(ServerVerification::CaStore(store.clone())),
                    ..TlsOptions::default()
                },
                "Server.CaStore",
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

        let config = ProxyOptions::None(Chosen).to_config().expect("no proxy");
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

    /// The system proxy's credentials are one credential over the defaults too: taken whole when
    /// the channel states none, and not at all when it states either.
    #[test]
    fn the_system_proxys_credentials_merge_whole() {
        let defaults = system(Some("alice"), Some("s3cret"));
        assert_eq!(system(None, None).over(&defaults), defaults);
        assert_eq!(
            system(Some("bob"), None).over(&defaults),
            system(Some("bob"), None),
            "another username takes none of the default's password"
        );
        assert_eq!(
            system(None, Some("other")).over(&defaults),
            system(None, Some("other"))
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
            idle_timeout_seconds: Some(Seconds(300.0)),
            simultaneous_calls_per_connection: Some(1),
            send: Http2SendOptions {
                coalescing_bytes: Some(0),
                stream_buffer_size: Some(4096),
                frames_per_write: Some(1),
            },
            receive: Some(Http2ReceiveOptions::Fixed(Http2FixedWindows {
                stream_window_size: Some(1024),
                connection_window_size: Some(65_535),
            })),
        }
        .to_config()
        .expect("admissible");
        assert_eq!(config.idle_timeout, Some(Duration::from_secs(300)));
        assert_eq!(config.simultaneous_calls_per_connection, Some(1));
        assert_eq!(config.write_coalescing, 0);
        assert_eq!(config.send_buffer, 4096);
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
                connection_window_size: Some(65_534),
                ..Http2FixedWindows::default()
            })),
            ..Http2Options::default()
        }
        .to_config()
        .expect_err("below the window every connection starts with");
        assert_eq!(refused.key(), "Receive.Fixed.ConnectionWindowSize");

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
                stream_buffer_size: Some(0),
                ..Http2SendOptions::default()
            },
            ..Http2Options::default()
        }
        .to_config()
        .expect_err("a buffer that never takes a byte");
        assert_eq!(refused.key(), "Send.StreamBufferSize");

        let refused = Http2Options {
            simultaneous_calls_per_connection: Some(0),
            ..Http2Options::default()
        }
        .to_config()
        .expect_err("a connection that carries no call");
        assert_eq!(refused.key(), "SimultaneousCallsPerConnection");

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
                keep_alive_while_idle: Some(true),
                receive: Some(Http2ReceiveOptions::Fixed(Http2FixedWindows {
                    stream_window_size: Some(70_000),
                    ..Http2FixedWindows::default()
                })),
                ..Http2Options::default()
            },
            ..ChannelOptions::default()
        };
        let merged = ChannelOptions {
            grpc: credits(3),
            http2: Http2Options {
                receive: Some(Http2ReceiveOptions::Fixed(Http2FixedWindows {
                    stream_window_size: Some(80_000),
                    ..Http2FixedWindows::default()
                })),
                ..Http2Options::default()
            },
            ..ChannelOptions::default()
        }
        .over(&defaults);

        assert_eq!(merged.grpc.user_agent.as_deref(), Some("default"));
        assert_eq!(merged.grpc.host.receive.window, Some(3));
        assert_eq!(merged.http2.keep_alive_while_idle, Some(true));
        assert_eq!(
            merged.http2.receive,
            Some(Http2ReceiveOptions::Fixed(Http2FixedWindows {
                stream_window_size: Some(80_000),
                ..Http2FixedWindows::default()
            }))
        );
    }

    /// The adaptive windows are an alternative to the fixed ones: either, stated over the other,
    /// replaces it, and a session that states neither keeps the default's.
    #[test]
    fn adaptive_windows_are_an_alternative_to_fixed_ones() {
        let adaptive = Http2Options {
            receive: Some(Http2ReceiveOptions::Adaptive(Chosen)),
            ..Http2Options::default()
        };
        let fixed = Http2Options {
            receive: Some(Http2ReceiveOptions::Fixed(Http2FixedWindows {
                stream_window_size: Some(70_000),
                ..Http2FixedWindows::default()
            })),
            ..Http2Options::default()
        };
        let unstated = Http2Options {
            keep_alive_while_idle: Some(true),
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

    /// An alternative stated over another is taken whole: nothing of the default's is combined
    /// into it. Beside it, every other option cumulates, the two backoff bounds included.
    #[test]
    fn a_stated_alternative_replaces_the_default_whole() {
        let mut url = ProxyUrl::new("http://proxy.test:3128");
        url.username = Some("someone".to_owned());
        let defaults = ChannelOptions {
            transport: TransportOptions {
                tls: TlsOptions {
                    server: Some(ServerVerification::CaPem("ca.pem".to_owned())),
                    client: Some(ClientCertificate::P12(P12Certificate::new("me.p12", None))),
                    override_target_name: Some("server".to_owned()),
                },
                proxy: Some(ProxyOptions::Url(url)),
                ..TransportOptions::default()
            },
            grpc: GrpcOptions {
                retry: RetryOptions {
                    max_backoff_seconds: Some(Seconds(5.0)),
                    ..RetryOptions::default()
                },
                ..GrpcOptions::default()
            },
            ..ChannelOptions::default()
        };
        let merged = ChannelOptions {
            transport: TransportOptions {
                tls: TlsOptions {
                    server: Some(ServerVerification::Unverified(Chosen)),
                    ..TlsOptions::default()
                },
                proxy: Some(ProxyOptions::None(Chosen)),
                ..TransportOptions::default()
            },
            grpc: GrpcOptions {
                retry: RetryOptions {
                    initial_backoff_seconds: Some(Seconds(10.0)),
                    ..RetryOptions::default()
                },
                ..GrpcOptions::default()
            },
            ..ChannelOptions::default()
        }
        .over(&defaults);

        let tls = &merged.transport.tls;
        assert_eq!(tls.server, Some(ServerVerification::Unverified(Chosen)));
        assert_eq!(
            tls.client,
            Some(ClientCertificate::P12(P12Certificate::new("me.p12", None))),
            "the identity is another alternative, which the channel leaves to its default"
        );
        assert_eq!(tls.override_target_name.as_deref(), Some("server"));
        assert_eq!(merged.transport.proxy, Some(ProxyOptions::None(Chosen)));
        assert_eq!(
            merged.grpc.retry.initial_backoff_seconds,
            Some(Seconds(10.0))
        );
        assert_eq!(merged.grpc.retry.max_backoff_seconds, Some(Seconds(5.0)));
    }

    /// An alternative stated over the same one merges its fields as a struct does, down to the
    /// alternative a field of it holds - but for credentials, which another target leaves behind.
    #[test]
    fn a_stated_alternative_over_the_same_one_merges_its_fields() {
        let mut default_url = ProxyUrl::new("http://default.test:3128");
        default_url.username = Some("alice".to_owned());
        default_url.password = Some(Password::new("s3cret"));
        let mut store = StoreCertificate::new(StoreSearch::FriendlyName("root".to_owned()));
        store.location = Some(StoreLocation::LocalMachine);
        let defaults = ChannelOptions {
            transport: TransportOptions {
                tls: TlsOptions {
                    server: Some(ServerVerification::CaStore(store)),
                    client: Some(ClientCertificate::P12(P12Certificate::new(
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
                    server: Some(ServerVerification::CaStore(own_store)),
                    client: Some(ClientCertificate::P12(P12Certificate::new("own.p12", None))),
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
        assert_eq!(
            url.password, None,
            "the default's password is for another proxy"
        );
        let Some(ServerVerification::CaStore(store)) = &merged.transport.tls.server else {
            panic!("{:?}", merged.transport.tls.server);
        };
        assert_eq!(store.find, StoreSearch::Thumbprint("ab".to_owned()));
        assert_eq!(store.name.as_deref(), Some("Pinned"));
        assert_eq!(store.location, Some(StoreLocation::LocalMachine));
        assert_eq!(
            merged.transport.tls.client,
            Some(ClientCertificate::P12(P12Certificate::new("own.p12", None))),
            "the default's password is for another bundle"
        );
    }

    /// Credentials stated for a target are taken for the same target: a proxy's for the same
    /// address, and only whole, a bundle's password for the same path.
    #[test]
    fn credentials_are_taken_for_the_target_they_were_stated_for() {
        let mut default_url = ProxyUrl::new("http://proxy.test:3128");
        default_url.username = Some("alice".to_owned());
        default_url.password = Some(Password::new("s3cret"));
        let defaults = ChannelOptions {
            transport: TransportOptions {
                tls: TlsOptions {
                    client: Some(ClientCertificate::P12(P12Certificate::new(
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
                        client: Some(ClientCertificate::P12(P12Certificate::new("me.p12", None))),
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
        assert_eq!(url.username.as_deref(), Some("alice"));
        assert_eq!(url.password, Some(Password::new("s3cret")));

        let Some(ProxyOptions::Url(url)) = merged_with(Some("bob")).transport.proxy else {
            panic!("a Url is merged into a Url");
        };
        assert_eq!(url.username.as_deref(), Some("bob"));
        assert_eq!(
            url.password, None,
            "another username takes none of the default's password"
        );
        assert_eq!(
            merged.transport.tls.client,
            Some(ClientCertificate::P12(P12Certificate::new(
                "me.p12",
                Some(Password::new("bundle"))
            )))
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
    /// admits, which the loader reads past: nothing would refuse it, so what this asserts is that
    /// nothing is logged as unknown.
    #[cfg(all(feature = "schema", feature = "configuration"))]
    #[test]
    fn every_option_the_schema_declares_is_one_serde_reads() {
        fn read_all<D: crate::configuration::Document + std::fmt::Debug>(rendered: &str) {
            let schema: serde_json::Value =
                serde_json::from_str(rendered).expect("the schema is a document");

            for alternative in 0..9 {
                let document = a_value_for(&schema, &schema, alternative);
                let logged = Logged::default();
                let subscriber = tracing_subscriber::fmt()
                    .with_writer(logged.clone())
                    .with_ansi(false)
                    .finish();

                let read = tracing::subscriber::with_default(subscriber, || {
                    crate::configuration::Configuration::with_prefix("")
                        .document(document.to_string())
                        .load::<D>()
                });

                assert!(
                    read.is_ok(),
                    "the schema declares {document}, which the loader refuses: {}",
                    read.unwrap_err()
                );
                let said = logged.said();
                assert!(
                    said.is_empty(),
                    "the schema declares {document}, which serde reads past: {said}"
                );
            }
        }

        read_all::<ChannelOptions>(&schema());
        read_all::<RuntimeOptions>(&runtime_schema());
    }

    /// What a subscriber writes, kept to be read back.
    #[cfg(all(feature = "schema", feature = "configuration"))]
    #[derive(Clone, Default)]
    struct Logged(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    #[cfg(all(feature = "schema", feature = "configuration"))]
    impl Logged {
        fn said(&self) -> String {
            let written = self.0.lock().unwrap_or_else(|held| held.into_inner());
            String::from_utf8_lossy(&written).into_owned()
        }
    }

    #[cfg(all(feature = "schema", feature = "configuration"))]
    impl std::io::Write for Logged {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(|held| held.into_inner())
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[cfg(all(feature = "schema", feature = "configuration"))]
    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Logged {
        type Writer = Self;

        fn make_writer(&'a self) -> Self {
            self.clone()
        }
    }

    /// A value each property of `node` admits, as one document naming all of them. A `oneOf`
    /// takes the alternative `alternative` picks in the base of its alternatives' count, and hands
    /// what is left of the index to the choices that alternative holds.
    ///
    /// Values rather than a name list, because an unknown name is logged and a value of the wrong
    /// type refused, and only a document carrying both exercises the two.
    #[cfg(all(feature = "schema", feature = "configuration"))]
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
