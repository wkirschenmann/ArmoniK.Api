//! A channel's options, settled into the engine's configuration.
//!
//! Every bound checked here is stated in the schema the options derive - a minimum, a maximum, a
//! minimum length - so a document a validator would reject is one this refuses too. They are
//! checked again rather than trusted: nothing obliges a host to have validated, and the engine is
//! what a bad value would break.

use std::fmt;
use std::time::Duration;

use hyper::Uri;

use crate::grpc::{AdaptiveConfig, GrpcChannelConfig, ReplayConfig, RetryConfig};
use crate::http2::{Http2Config, ProxyConfig, TcpConfig, TlsConfig, TransportConfig};
use crate::options::{
    ChannelOptions, Deadline, OptionRefusal, ProxyOptions, RetryOptions, Seconds, SendCompression,
    TcpKeepalive, ThrottleOptions, LARGEST_WINDOW,
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
    send_limit: Option<usize>,
    receive_limit: Option<usize>,
    tls: TlsConfig,
    tcp: TcpConfig,
    http2: Http2Config,
    proxy: ProxyConfig,
    retry: Option<RetryConfig>,
    adaptive: Option<AdaptiveConfig>,
    replay: ReplayConfig,
    accept_encodings: Vec<crate::grpc::Encoding>,
}

impl ChannelSettings {
    /// Settles the options, refusing what the schema refuses, and saying over which key.
    ///
    /// Options that cannot hold together are refused as well, naming each: a channel's options are
    /// settled once they are merged, and then nothing can state another.
    pub fn settle(options: ChannelOptions) -> Result<Self, SettingRefusal> {
        let (settled, incoherent) = Self::settle_leniently(options)?;
        if incoherent.is_empty() {
            Ok(settled)
        } else {
            Err(SettingRefusal::Incoherent(incoherent))
        }
    }

    /// Admits the channel defaults of a runtime, which a channel's own options can still
    /// override: a value wrong by itself is refused, and what cannot hold together is said in the
    /// log, naming each key.
    pub fn settle_defaults(options: ChannelOptions) -> Result<(), SettingRefusal> {
        let (_, incoherent) = Self::settle_leniently(options)?;
        for incoherence in &incoherent {
            tracing::warn!(
                "the channel defaults of the runtime are incoherent, which a channel can still \
                 override: {incoherence}"
            );
        }
        Ok(())
    }

    /// The settings, and what cannot hold together instead of a refusal for it.
    fn settle_leniently(
        options: ChannelOptions,
    ) -> Result<(Self, Vec<OptionRefusal>), SettingRefusal> {
        let window = |key: &'static str, asked: Option<i32>| match asked {
            Some(value) if !(1..=LARGEST_WINDOW).contains(&value) => {
                Err(SettingRefusal::Window { key, value })
            }
            _ => Ok(()),
        };
        let grpc = &options.grpc;
        window("Grpc.Host.Receive.Window", grpc.host.receive.window)?;
        window("Grpc.Host.Send.Window", grpc.host.send.window)?;

        // A limit of 0 KiB is refused: it admits only empty messages, which is a channel with no
        // use.
        let send_limit = match &grpc.send.message_size_kib {
            None => None,
            Some(size) => size.limit().map_err(|refused| {
                SettingRefusal::Option(refused.under("Grpc.Send.MessageSizeKiB"))
            })?,
        };
        let receive_limit = match &grpc.receive.message_size_kib {
            None => None,
            Some(size) => Some(size.limit().map_err(|refused| {
                SettingRefusal::Option(refused.under("Grpc.Receive.MessageSizeKiB"))
            })?),
        };

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
        let default_deadline = match &grpc.deadline {
            None | Some(Deadline::None) => None,
            Some(Deadline::Default(seconds)) => duration("Grpc.Deadline.Default", Some(*seconds))?,
        };

        let tls = options
            .transport
            .tls
            .load()
            .map_err(|refused| SettingRefusal::Option(refused.under("Transport.Tls")))?;
        let mut incoherent = Vec::new();
        let mut converted = |found: Vec<OptionRefusal>, unit: &str| {
            incoherent.extend(found.into_iter().map(|refused| refused.under(unit)));
        };
        let tcp = options
            .transport
            .tcp_keepalive
            .as_ref()
            .map_or_else(|| Ok(TcpConfig::default()), TcpKeepalive::to_config)
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
        let traffic = &grpc.outbound_traffic;
        let (retry, found) = traffic
            .retry
            .as_ref()
            .map_or_else(|| RetryOptions::default().convert(), RetryOptions::convert)
            .map_err(|refused| {
                SettingRefusal::Option(refused.under("Grpc.OutboundTraffic.Retry"))
            })?;
        converted(found, "Grpc.OutboundTraffic.Retry");
        let adaptive = traffic
            .throttle
            .as_ref()
            .map_or_else(
                || ThrottleOptions::default().to_config(),
                ThrottleOptions::to_config,
            )
            .map_err(|refused| {
                SettingRefusal::Option(refused.under("Grpc.OutboundTraffic.Throttle"))
            })?;
        let replay = traffic.replay.to_config().map_err(|refused| {
            SettingRefusal::Option(refused.under("Grpc.OutboundTraffic.Replay"))
        })?;
        let accept_encodings = grpc.receive.accepted_encodings();

        Ok((
            Self {
                options,
                connect_timeout,
                default_deadline,
                send_limit,
                receive_limit,
                tls,
                tcp,
                http2,
                proxy,
                retry,
                adaptive,
                replay,
                accept_encodings,
            },
            incoherent,
        ))
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
        config.max_send_message_size = self.send_limit;
        if let Some(max) = self.receive_limit {
            config.max_recv_message_size = max;
        }
        config.send_encoding = grpc.send.compression.and_then(SendCompression::encoding);
        config.accept_encodings = self.accept_encodings;
        if let Some(bytes) = grpc.host.receive.coalescing_bytes {
            config.delivery_coalescing = bytes as usize;
        }
        config.default_deadline = self.default_deadline;
        config.retry = self.retry;
        config.adaptive = self.adaptive;
        config.replay = self.replay;
        config
    }
}

/// Why a channel's options were refused, named by the key they were refused over.
#[derive(Debug)]
#[non_exhaustive]
pub enum SettingRefusal {
    /// A window outside what the schema admits.
    Window { key: &'static str, value: i32 },
    /// A count of bytes below zero.
    Bytes { key: &'static str, value: i32 },
    /// An empty user agent.
    EmptyUserAgent,
    /// A duration no `Duration` holds, or one below what it holds.
    Seconds { key: &'static str, seconds: f64 },
    /// An option of a unit the engine converts, a file it names included.
    Option(OptionRefusal),
    /// Options that are each valid and cannot hold together, each incoherence naming its keys.
    Incoherent(Vec<OptionRefusal>),
}

impl fmt::Display for SettingRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Window { key, value } => write!(
                f,
                "{key} is {value}, and has to be between 1 and {LARGEST_WINDOW}"
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
            Self::Incoherent(found) => {
                for (index, refused) in found.iter().enumerate() {
                    if index > 0 {
                        f.write_str("; ")?;
                    }
                    refused.fmt(f)?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for SettingRefusal {}
