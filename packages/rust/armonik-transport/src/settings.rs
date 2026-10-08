//! A channel's options, settled into the engine's configuration.
//!
//! Every bound checked here is stated in the schema the options derive - a minimum, a maximum, a
//! minimum length - so a document a validator would reject is one this refuses too. They are
//! checked again rather than trusted: nothing obliges a host to have validated, and the engine is
//! what a bad value would break.

use std::fmt;
use std::time::Duration;

use hyper::Uri;

use crate::grpc::{GrpcChannelConfig, RateLimitConfig, RetryConfig};
use crate::http2::{Http2Config, ProxyConfig, TcpConfig, TlsConfig, TransportConfig};
use crate::options::{
    ChannelOptions, MessageEncoding, OptionRefusal, ProxyOptions, Seconds, LARGEST_WINDOW,
};

// What a configuration that names neither gets. One send, the smallest window. Four deliveries:
// the head and the message of a unary call each take one, and a stream has the rest.
const MAX_SENDS_IN_FLIGHT: i32 = 1;
const DELIVERY_CREDITS: i32 = 4;

/// A channel's options, read and found admissible.
///
/// Each unit is held as the engine configuration it became rather than as what was written:
/// converting once, where the options are refused, is what leaves nothing here that can fail -
/// and the files the TLS unit names are read there, once.
pub struct ChannelSettings {
    options: ChannelOptions,
    connect_timeout: Option<Duration>,
    default_deadline: Option<Duration>,
    tls: TlsConfig,
    tcp: TcpConfig,
    http2: Http2Config,
    proxy: ProxyConfig,
    retry: RetryConfig,
    rate_limit: Option<RateLimitConfig>,
}

impl ChannelSettings {
    /// Settles the options, refusing exactly what the schema refuses, and saying over which key.
    pub fn settle(options: ChannelOptions) -> Result<Self, SettingRefusal> {
        let window = |key: &'static str, asked: Option<i32>| match asked {
            Some(value) if !(1..=LARGEST_WINDOW).contains(&value) => {
                Err(SettingRefusal::Window { key, value })
            }
            _ => Ok(()),
        };
        let grpc = &options.grpc;
        window("Grpc.Host.Receive.Window", grpc.host.receive.window)?;
        window("Grpc.Host.Send.Window", grpc.host.send.window)?;

        // Zero is refused: it admits only empty messages, which is a channel with no use.
        for (key, max) in [
            ("Grpc.Send.MaxMessageSize", grpc.send.max_message_size),
            ("Grpc.Receive.MaxMessageSize", grpc.receive.max_message_size),
        ] {
            if let Some(value) = max.filter(|max| *max < 1) {
                return Err(SettingRefusal::NoMessage { key, value });
            }
        }

        if let Some(value) = grpc
            .host
            .receive
            .coalescing_bytes
            .filter(|bytes| *bytes < 0)
        {
            return Err(SettingRefusal::Bytes {
                key: "Grpc.Host.Receive.CoalescingBytes",
                value,
            });
        }

        if grpc.user_agent.as_deref().is_some_and(str::is_empty) {
            return Err(SettingRefusal::EmptyUserAgent);
        }

        // Below a nanosecond is refused, as the schema's `minimum` refuses it: `Duration` holds
        // nothing finer, so the conversion could round it to zero, which no dial or call could
        // beat. And a number is not yet a duration: `Duration` holds no value past its own range
        // either, so the conversion is what says whether the options named one.
        let duration = |key: &'static str, asked: Option<Seconds>| match asked {
            None => Ok(None),
            Some(seconds) => {
                let refused = SettingRefusal::Seconds {
                    key,
                    seconds: seconds.0,
                };
                if seconds.0 < 1e-9 {
                    return Err(refused);
                }
                Duration::try_from(seconds).map(Some).map_err(|_| refused)
            }
        };
        let connect_timeout = duration(
            "Transport.ConnectTimeoutSeconds",
            options.transport.connect_timeout_seconds,
        )?;
        // Zero states that a call has none, over a deadline an earlier source set.
        let default_deadline = duration(
            "Grpc.DefaultDeadlineSeconds",
            grpc.default_deadline_seconds
                .filter(|seconds| !crate::options::is_off(*seconds)),
        )?;

        let tls = options
            .transport
            .tls
            .load()
            .map_err(|refused| SettingRefusal::Option(refused.under("Transport.Tls")))?;
        let tcp = options
            .transport
            .tcp_keepalive
            .to_config()
            .map_err(|refused| SettingRefusal::Option(refused.under("Transport.TcpKeepalive")))?;
        let http2 = options
            .http2
            .to_config()
            .map_err(|refused| SettingRefusal::Option(refused.under("Http2")))?;
        let proxy = options
            .transport
            .proxy
            .as_ref()
            .map_or_else(
                || ProxyOptions::default().to_config(),
                ProxyOptions::to_config,
            )
            .map_err(|refused| SettingRefusal::Option(refused.under("Transport.Proxy")))?;
        let retry = grpc
            .retry
            .to_config()
            .map_err(|refused| SettingRefusal::Option(refused.under("Grpc.Retry")))?;
        let rate_limit = grpc
            .rate
            .limit
            .to_config()
            .map_err(|refused| SettingRefusal::Option(refused.under("Grpc.Rate.Limit")))?;

        Ok(Self {
            options,
            connect_timeout,
            default_deadline,
            tls,
            tcp,
            http2,
            proxy,
            retry,
            rate_limit,
        })
    }

    /// How many of a call's payloads the host may hold at once.
    pub fn delivery_credits(&self) -> usize {
        self.options
            .grpc
            .host
            .receive
            .window
            .unwrap_or(DELIVERY_CREDITS) as usize
    }

    /// Whether the channel dials as it is created.
    pub fn connect_eagerly(&self) -> bool {
        self.options.transport.connect_eagerly.unwrap_or(false)
    }

    /// How many messages a call may have sent and unacquitted at once.
    pub fn max_sends_in_flight(&self) -> usize {
        self.options
            .grpc
            .host
            .send
            .window
            .unwrap_or(MAX_SENDS_IN_FLIGHT) as usize
    }

    /// The engine's configuration of a channel to `endpoint`.
    pub fn into_channel_config(self, endpoint: Uri) -> GrpcChannelConfig {
        let max_sends_in_flight = self.max_sends_in_flight();
        let mut transport = TransportConfig::new(endpoint);
        if let Some(connect_timeout) = self.connect_timeout {
            transport.connect_timeout = connect_timeout;
        }
        transport.tls = self.tls;
        transport.tcp = self.tcp;
        transport.http2 = self.http2;
        transport.proxy = self.proxy;

        let mut config = GrpcChannelConfig::new(transport);
        config.max_sends_in_flight = max_sends_in_flight;
        let grpc = self.options.grpc;
        config.user_agent = grpc.user_agent;
        config.max_send_message_size = grpc.send.max_message_size.map(|max| max as usize);
        if let Some(max) = grpc.receive.max_message_size {
            config.max_recv_message_size = max as usize;
        }
        config.send_encoding = grpc.send.compression.map(MessageEncoding::encoding);
        config.accept_encodings = grpc
            .receive
            .compression
            .iter()
            .flatten()
            .copied()
            .map(MessageEncoding::encoding)
            .collect();
        if let Some(bytes) = grpc.host.receive.coalescing_bytes {
            config.delivery_coalescing = bytes as usize;
        }
        config.default_deadline = self.default_deadline;
        config.retry = Some(self.retry);
        config.rate_limit = self.rate_limit;
        config
    }
}

/// Why a channel's options were refused, named by the key they were refused over.
#[derive(Debug)]
#[non_exhaustive]
pub enum SettingRefusal {
    /// A window outside what the schema admits.
    Window { key: &'static str, value: i32 },
    /// A message size limit that admits only empty messages.
    NoMessage { key: &'static str, value: i32 },
    /// A count of bytes below zero.
    Bytes { key: &'static str, value: i32 },
    /// An empty user agent.
    EmptyUserAgent,
    /// A duration no `Duration` holds, or one below what it holds.
    Seconds { key: &'static str, seconds: f64 },
    /// An option of a unit the engine converts, a file it names included.
    Option(OptionRefusal),
}

impl fmt::Display for SettingRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Window { key, value } => write!(
                f,
                "{key} is {value}, and has to be between 1 and {LARGEST_WINDOW}"
            ),
            Self::NoMessage { key, value } => write!(
                f,
                "{key} is {value}, and has to be at least 1 - zero admits only empty messages"
            ),
            Self::Bytes { key, value } => write!(f, "{key} is {value}, and has to be at least 0"),
            Self::EmptyUserAgent => {
                f.write_str("Grpc.UserAgent is empty, and has to name something")
            }
            Self::Seconds { key, seconds } => write!(
                f,
                "{key} is {seconds}, and has to be at least 1e-9 and less than 2^64"
            ),
            Self::Option(refused) => refused.fmt(f),
        }
    }
}

impl std::error::Error for SettingRefusal {}
