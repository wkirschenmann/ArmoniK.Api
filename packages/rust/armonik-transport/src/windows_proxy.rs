//! The proxy the current user's Windows network settings name: a PAC script, found by WPAD or at
//! the configured address and run by WinHTTP, else the manual proxy with its bypass list.

use std::ffi::c_void;
use std::time::Duration;

use http::Uri;
use windows_sys::core::PWSTR;
use windows_sys::Win32::Foundation::GlobalFree;
use windows_sys::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpGetIEProxyConfigForCurrentUser, WinHttpGetProxyForUrl, WinHttpOpen,
    WinHttpSetTimeouts, WINHTTP_ACCESS_TYPE_NO_PROXY, WINHTTP_AUTOPROXY_AUTO_DETECT,
    WINHTTP_AUTOPROXY_CONFIG_URL, WINHTTP_AUTOPROXY_OPTIONS, WINHTTP_AUTO_DETECT_TYPE_DHCP,
    WINHTTP_AUTO_DETECT_TYPE_DNS_A, WINHTTP_CURRENT_USER_IE_PROXY_CONFIG, WINHTTP_PROXY_INFO,
};

/// The settings as `WinHttpGetIEProxyConfigForCurrentUser` reads them.
#[derive(Default)]
pub(crate) struct Settings {
    pub(crate) auto_detect: bool,
    pub(crate) auto_config_url: Option<String>,
    /// `host:port` for every scheme, or `scheme=host:port` entries, separated by `;` or
    /// whitespace.
    pub(crate) proxy: Option<String>,
    /// Hosts separated by `;`, with `*` wildcards, and `<local>` for any name with no dot.
    pub(crate) bypass: Option<String>,
}

/// Says which settings are set and not what they hold: a manual entry or a script's address may
/// carry `user:password@`.
impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("auto_detect", &self.auto_detect)
            .field("auto_config_url", &self.auto_config_url.is_some())
            .field("proxy", &self.proxy.is_some())
            .field("bypass", &self.bypass)
            .finish()
    }
}

impl Settings {
    /// The current user's. A user with no profile, as a service may run as, has none, and is
    /// dialled directly.
    pub(crate) fn current_user() -> Self {
        let mut config = WINHTTP_CURRENT_USER_IE_PROXY_CONFIG::default();
        // SAFETY: `config` is a valid out-parameter, and its strings are this call's to free.
        if unsafe { WinHttpGetIEProxyConfigForCurrentUser(&mut config) } == 0 {
            return Self::default();
        }
        // SAFETY: each string is WinHTTP's allocation, or null, and is freed once.
        unsafe {
            Self {
                auto_detect: config.fAutoDetect != 0,
                auto_config_url: taken(config.lpszAutoConfigUrl),
                proxy: taken(config.lpszProxy),
                bypass: taken(config.lpszProxyBypass),
            }
        }
    }

    fn is_automatic(&self) -> bool {
        self.auto_detect || self.auto_config_url.is_some()
    }

    pub(crate) fn names_a_proxy(&self) -> bool {
        self.is_automatic() || self.proxy.is_some()
    }
}

/// A string WinHTTP allocated, copied and freed; none for a null or empty one.
///
/// # Safety
///
/// `text` is null, or a NUL-terminated `GlobalAlloc` allocation freed nowhere else.
unsafe fn taken(text: PWSTR) -> Option<String> {
    if text.is_null() {
        return None;
    }
    let len = (0..).take_while(|&at| *text.add(at) != 0).count();
    let value = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
    GlobalFree(text.cast());
    (!value.is_empty()).then_some(value)
}

/// A WinHTTP session that only resolves proxies, so it is opened with none of its own.
struct Session(*mut c_void);

// SAFETY: a WinHTTP session handle may be used from any thread, and `WinHttpGetProxyForUrl` may
// run on one concurrently.
unsafe impl Send for Session {}
// SAFETY: as above.
unsafe impl Sync for Session {}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: the handle is this session's, closed once.
        unsafe { WinHttpCloseHandle(self.0) };
    }
}

pub(crate) struct WindowsProxy {
    settings: Settings,
    /// Open when the settings are automatic. A session that cannot be opened leaves the manual
    /// proxy to decide.
    session: Option<Session>,
}

impl std::fmt::Debug for WindowsProxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowsProxy")
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

impl WindowsProxy {
    /// `timeout` bounds each step of a PAC script's download.
    pub(crate) fn new(settings: Settings, timeout: Duration) -> Self {
        let session = if settings.is_automatic() {
            open(timeout)
        } else {
            None
        };
        Self { settings, session }
    }

    /// The proxy entry a dial of `target` goes through, as the settings write it - `host:port`,
    /// perhaps with a scheme - or none for a direct dial. Blocks while WinHTTP finds and runs a
    /// PAC script; a script that cannot be found or run leaves the manual proxy to decide. A
    /// manual `socks=` entry, the only one there is for the scheme, comes back as `socks://`,
    /// which the connector refuses.
    pub(crate) fn resolve(&self, target: &Uri) -> Option<String> {
        if let Some(resolved) = self.automatic(target) {
            return resolved;
        }
        let host = target.host().unwrap_or_default();
        let host = host.trim_start_matches('[').trim_end_matches(']');
        if self
            .settings
            .bypass
            .as_deref()
            .is_some_and(|bypass| bypassed(bypass, host))
        {
            return None;
        }
        let scheme = target.scheme_str().unwrap_or("http");
        let proxy = self.settings.proxy.as_deref()?;
        for_scheme(proxy, scheme)
            .map(str::to_owned)
            .or_else(|| for_scheme(proxy, "socks").map(|socks| format!("socks://{socks}")))
    }

    /// What the PAC script says, or none when there is no script or it could not be run. WinHTTP
    /// remembers a script it could not fetch, so the dials that follow do not wait on it again.
    fn automatic(&self, target: &Uri) -> Option<Option<String>> {
        let session = self.session.as_ref()?;
        let url = wide(&target.to_string());
        let config_url = self.settings.auto_config_url.as_deref().map(wide);
        let mut options = WINHTTP_AUTOPROXY_OPTIONS::default();
        if self.settings.auto_detect {
            options.dwFlags |= WINHTTP_AUTOPROXY_AUTO_DETECT;
            options.dwAutoDetectFlags =
                WINHTTP_AUTO_DETECT_TYPE_DHCP | WINHTTP_AUTO_DETECT_TYPE_DNS_A;
        }
        if let Some(config_url) = &config_url {
            options.dwFlags |= WINHTTP_AUTOPROXY_CONFIG_URL;
            options.lpszAutoConfigUrl = config_url.as_ptr();
        }
        // A script served behind Windows authentication is fetched as the user.
        options.fAutoLogonIfChallenged = 1;
        let mut info = WINHTTP_PROXY_INFO::default();
        // SAFETY: the strings outlive the call, and `info`'s are this call's to free.
        let found =
            unsafe { WinHttpGetProxyForUrl(session.0, url.as_ptr(), &mut options, &mut info) } != 0;
        // SAFETY: as above, each freed once.
        let (proxy, _) = unsafe { (taken(info.lpszProxy), taken(info.lpszProxyBypass)) };
        if !found {
            return None;
        }
        if info.dwAccessType == WINHTTP_ACCESS_TYPE_NO_PROXY {
            return Some(None);
        }
        // A list such as `a:1; b:2`, of which a dial tries the first. WinHTTP writes an `HTTPS`
        // answer as an `https://` URL, a scheme to refuse, and drops a `SOCKS` one.
        Some(proxy.and_then(|proxy| {
            proxy
                .split([';', ' '])
                .find(|entry| !entry.is_empty())
                .map(str::to_owned)
        }))
    }
}

fn open(timeout: Duration) -> Option<Session> {
    // SAFETY: no agent, proxy or bypass string, and no flag: a synchronous session.
    let handle = unsafe {
        WinHttpOpen(
            std::ptr::null(),
            WINHTTP_ACCESS_TYPE_NO_PROXY,
            std::ptr::null(),
            std::ptr::null(),
            0,
        )
    };
    if handle.is_null() {
        return None;
    }
    let session = Session(handle);
    // At least 1: WinHTTP reads 0 as no timeout at all.
    let millis = i32::try_from(timeout.as_millis())
        .unwrap_or(i32::MAX)
        .max(1);
    // SAFETY: the handle is open.
    unsafe { WinHttpSetTimeouts(session.0, millis, millis, millis, millis) };
    Some(session)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// The entry for `scheme` in a manual proxy list: its own `scheme=` one, else one naming no
/// scheme.
fn for_scheme<'a>(proxy: &'a str, scheme: &str) -> Option<&'a str> {
    let entries = || {
        proxy
            .split(|c: char| c == ';' || c.is_whitespace())
            .filter(|entry| !entry.is_empty())
    };
    entries()
        .find_map(|entry| {
            let (named, value) = entry.split_once('=')?;
            named.eq_ignore_ascii_case(scheme).then_some(value)
        })
        .or_else(|| entries().find(|entry| !entry.contains('=')))
}

/// Whether the bypass list names `host`. An entry may carry a scheme and a port, which are not
/// matched, and `*` for any run of characters.
fn bypassed(bypass: &str, host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    bypass
        .split([';', ' ', ','])
        .filter(|entry| !entry.is_empty())
        .any(|entry| {
            if entry.eq_ignore_ascii_case("<local>") {
                return !host.contains('.') && !host.contains(':');
            }
            let entry = entry.split_once("://").map_or(entry, |(_, rest)| rest);
            let is_port = |port: &str| port.bytes().all(|byte| byte.is_ascii_digit());
            // A bracketed address keeps its own colons; a port follows the bracket.
            let entry = match entry
                .strip_prefix('[')
                .and_then(|inner| inner.split_once(']'))
            {
                Some((address, after))
                    if after.is_empty() || after.strip_prefix(':').is_some_and(is_port) =>
                {
                    address
                }
                _ => match entry.rsplit_once(':') {
                    Some((name, port)) if !name.contains(':') && is_port(port) => name,
                    _ => entry,
                },
            };
            glob(&entry.to_ascii_lowercase(), &host)
        })
}

fn glob(pattern: &str, text: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == text,
        Some((head, tail)) => {
            let Some(rest) = text.strip_prefix(head) else {
                return false;
            };
            (0..=rest.len())
                .filter(|&at| rest.is_char_boundary(at))
                .any(|at| glob(tail, &rest[at..]))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn a_manual_list_gives_each_scheme_its_own_entry_or_the_shared_one() {
        assert_eq!(
            for_scheme("proxy.test:3128", "https"),
            Some("proxy.test:3128")
        );
        let list = "http=plain.test:80; https=secure.test:443";
        assert_eq!(for_scheme(list, "http"), Some("plain.test:80"));
        assert_eq!(for_scheme(list, "HTTPS"), Some("secure.test:443"));
        assert_eq!(for_scheme("ftp=files.test:21", "https"), None);
        assert_eq!(
            for_scheme("socks=socks.test:1080;shared.test:8080", "https"),
            Some("shared.test:8080")
        );
        assert_eq!(
            for_scheme("http=plain.test:80 https=secure.test:443", "https"),
            Some("secure.test:443")
        );
    }

    #[test]
    fn the_bypass_list_matches_names_wildcards_and_local_hosts() {
        let list = "<local>;*.corp.test;10.*;http://exact.test:8080";
        for host in [
            "intranet",
            "a.corp.test",
            "A.Corp.Test",
            "10.1.2.3",
            "exact.test",
        ] {
            assert!(bypassed(list, host), "{host}");
        }
        for host in ["corp.test", "outside.test", "110.1.2.3", "::1"] {
            assert!(!bypassed(list, host), "{host}");
        }
        for entry in ["[::1]", "[::1]:8080", "::1"] {
            assert!(bypassed(entry, "::1"), "{entry}");
        }
    }

    #[test]
    fn manual_settings_route_by_scheme_unless_bypassed() {
        let proxy = WindowsProxy::new(
            Settings {
                proxy: Some("http=plain.test:80;https=secure.test:443".to_owned()),
                bypass: Some("*.corp.test".to_owned()),
                ..Settings::default()
            },
            Duration::from_secs(5),
        );
        let resolve = |target: &'static str| proxy.resolve(&Uri::from_static(target));
        assert_eq!(
            resolve("http://server.test:1").as_deref(),
            Some("plain.test:80")
        );
        assert_eq!(
            resolve("https://server.test:1").as_deref(),
            Some("secure.test:443")
        );
        assert_eq!(resolve("https://a.corp.test:1"), None);
        let socks_only = WindowsProxy::new(
            Settings {
                proxy: Some("socks=socks.test:1080".to_owned()),
                ..Settings::default()
            },
            Duration::from_secs(5),
        );
        assert_eq!(
            socks_only
                .resolve(&Uri::from_static("http://server.test:1"))
                .as_deref(),
            Some("socks://socks.test:1080")
        );
        assert!(!Settings::default().names_a_proxy());
    }

    /// One HTTP server thread answering every request with `body`.
    fn serve(body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("an address");
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut request = [0u8; 4096];
                let _ = stream.read(&mut request);
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/x-ns-proxy-autoconfig\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        format!("http://{address}/proxy.pac")
    }

    const PAC: &str = r#"function FindProxyForURL(url, host) {
        if (host == "direct.test") return "DIRECT";
        if (host == "socks.test") return "SOCKS 127.0.0.1:1080";
        if (host == "https.test") return "HTTPS 127.0.0.1:443";
        return "PROXY 127.0.0.1:3128; PROXY 127.0.0.1:3129";
    }"#;

    /// WinHTTP itself runs the script, as it does for a setting the user made.
    #[test]
    fn a_pac_script_decides_and_its_first_proxy_is_taken() {
        let proxy = WindowsProxy::new(
            Settings {
                auto_config_url: Some(serve(PAC)),
                proxy: Some("manual.test:8080".to_owned()),
                ..Settings::default()
            },
            Duration::from_secs(10),
        );
        assert_eq!(
            proxy
                .resolve(&Uri::from_static("http://server.test:1"))
                .as_deref(),
            Some("127.0.0.1:3128")
        );
        assert_eq!(
            proxy.resolve(&Uri::from_static("http://direct.test:1")),
            None
        );
        let https = proxy
            .resolve(&Uri::from_static("http://https.test:1"))
            .expect("a proxy");
        assert!(https.starts_with("https://"), "{https}");
        // WinHTTP drops a `SOCKS` answer, which leaves a direct dial.
        assert_eq!(
            proxy.resolve(&Uri::from_static("http://socks.test:1")),
            None
        );
    }

    #[test]
    fn a_debug_print_says_which_settings_are_set_and_not_what_they_hold() {
        let printed = format!(
            "{:?}",
            WindowsProxy::new(
                Settings {
                    auto_config_url: Some("http://alice:s3cret@pac.test/proxy.pac".to_owned()),
                    proxy: Some("alice:s3cret@proxy.test:3128".to_owned()),
                    ..Settings::default()
                },
                Duration::from_secs(1),
            )
        );
        assert!(!printed.contains("s3cret"), "{printed}");
        assert!(printed.contains("proxy: true"), "{printed}");
        assert!(printed.contains("auto_config_url: true"), "{printed}");
    }

    /// A server that never answers holds the first resolution up to the timeout; WinHTTP does
    /// not make the second wait on it again.
    #[test]
    fn a_failed_script_is_not_tried_again_at_once() {
        use std::time::Instant;
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("an address");
        std::thread::spawn(move || {
            let held: Vec<_> = listener.incoming().flatten().collect();
            drop(held);
        });
        let timeout = Duration::from_secs(1);
        let proxy = WindowsProxy::new(
            Settings {
                auto_config_url: Some(format!("http://{address}/silent.pac")),
                proxy: Some("manual.test:8080".to_owned()),
                ..Settings::default()
            },
            timeout,
        );
        let resolve = |target: &'static str| {
            let started = Instant::now();
            let resolved = proxy.resolve(&Uri::from_static(target));
            (resolved, started.elapsed())
        };
        let (resolved, _) = resolve("http://server.test:1");
        assert_eq!(resolved.as_deref(), Some("manual.test:8080"));
        let (resolved, waited) = resolve("http://other.test:1");
        assert_eq!(resolved.as_deref(), Some("manual.test:8080"));
        assert!(
            waited < timeout / 2,
            "the script was asked again: {waited:?}"
        );
    }

    #[test]
    fn a_pac_script_out_of_reach_leaves_the_manual_proxy_to_decide() {
        let closed = {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            listener.local_addr().expect("an address")
        };
        let proxy = WindowsProxy::new(
            Settings {
                auto_config_url: Some(format!("http://{closed}/proxy.pac")),
                proxy: Some("manual.test:8080".to_owned()),
                ..Settings::default()
            },
            Duration::from_secs(5),
        );
        assert_eq!(
            proxy
                .resolve(&Uri::from_static("http://server.test:1"))
                .as_deref(),
            Some("manual.test:8080")
        );
    }
}
