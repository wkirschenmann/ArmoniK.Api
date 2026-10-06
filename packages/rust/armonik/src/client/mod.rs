//! ArmoniK clients for all the services

use snafu::{ResultExt, Snafu};

// Re-exported here, so a caller reaches them through the client rather than through the transport
// crate.
#[cfg(feature = "_gen-client")]
pub use armonik_transport::grpc::{GrpcChannel, GrpcStatus, GrpcStatusCode};
#[cfg(feature = "_gen-client")]
pub use armonik_transport::{ClientConfig, ClientConfigArgs, ConfigError, ReadEnvError};

#[cfg(feature = "_gen-client")]
pub mod rpc;

#[cfg(feature = "worker")]
mod agent;
#[cfg(feature = "client")]
mod applications;
#[cfg(feature = "client")]
mod auth;
#[cfg(feature = "client")]
mod events;
#[cfg(feature = "client")]
mod health_checks;
#[cfg(feature = "client")]
mod partitions;
#[cfg(feature = "client")]
mod results;
#[cfg(feature = "client")]
mod sessions;
#[cfg(feature = "client")]
mod submitter;
#[cfg(feature = "client")]
mod tasks;
#[cfg(feature = "client")]
mod versions;
#[cfg(feature = "agent")]
mod worker;

#[cfg(feature = "worker")]
pub use agent::Agent;
#[cfg(feature = "client")]
pub use applications::Applications;
#[cfg(feature = "client")]
pub use auth::Auth;
#[cfg(feature = "client")]
pub use events::Events;
#[cfg(feature = "client")]
pub use health_checks::HealthChecks;
#[cfg(feature = "client")]
pub use partitions::Partitions;
#[cfg(feature = "client")]
pub use results::Results;
#[cfg(feature = "client")]
pub use sessions::Sessions;
#[cfg(feature = "client")]
#[allow(deprecated)]
pub use submitter::Submitter;
#[cfg(feature = "client")]
pub use tasks::Tasks;
#[cfg(feature = "client")]
pub use versions::Versions;
#[cfg(feature = "agent")]
pub use worker::Worker;

/// ArmoniK Client, over the engine's channel.
#[derive(Clone)]
pub struct Client {
    channel: GrpcChannel,
}

/// Why a client could not be built.
#[derive(Debug, Snafu)]
#[non_exhaustive]
pub enum ConnectionError {
    #[snafu(display("the client's configuration is refused"))]
    #[non_exhaustive]
    Config { source: ConfigError },
    #[snafu(display("the channel's configuration is refused"))]
    #[non_exhaustive]
    Channel {
        source: armonik_transport::grpc::GrpcChannelConfigError,
    },
    #[snafu(display("the channel could not connect"))]
    #[non_exhaustive]
    Connect {
        source: armonik_transport::grpc::ChannelError,
    },
}

impl Client {
    /// Create a new client using the configuration from the environment variables
    pub async fn new() -> Result<Self, ConnectionError> {
        Self::with_config(ClientConfig::from_env().context(ConfigSnafu {})?).await
    }

    /// Create a new client with the specified client configuration, connected once it returns.
    ///
    /// The client's connections run on the tokio runtime current here, which must outlive it.
    pub async fn with_config(config: ClientConfig) -> Result<Self, ConnectionError> {
        // Rendered rather than printed: `endpoint` is a public field, so a config built by hand
        // never met the check that refuses `user:password@`, and this span is a log line.
        let endpoint = armonik_transport::safe_endpoint(&config.endpoint);
        tracing_futures::Instrument::instrument(
            async move {
                let channel = GrpcChannel::new(
                    config.channel_config().context(ConfigSnafu {})?,
                    tokio::runtime::Handle::current(),
                )
                .context(ChannelSnafu {})?;
                channel.connect().await.context(ConnectSnafu {})?;
                Ok(Self::with_channel(channel))
            },
            tracing::debug_span!("Client", endpoint),
        )
        .await
    }

    #[cfg(test)]
    async fn get_nb_request(service: &str, rpc: &str) -> usize {
        use std::collections::HashMap;

        use http_body_util::BodyExt;
        use hyper_util::rt::TokioExecutor;

        let mut config = ClientConfig::from_env().unwrap();

        match std::env::var("Http__Endpoint") {
            Ok(value) if !value.is_empty() => {
                config.endpoint = hyper::Uri::try_from(value).expect("HTTP endpoint");
            }
            Ok(_) | Err(std::env::VarError::NotPresent) => {}
            Err(std::env::VarError::NotUnicode(value)) => {
                panic!("{value:?} is not a valid unicode string")
            }
        }

        let request = hyper::Request::get(format!("{}calls.json", config.endpoint))
            .body(http_body_util::Empty::<&[u8]>::new())
            .expect("Request");

        let https = armonik_transport::https_connector(config)
            .await
            .expect("Build connection information");

        let client = hyper_util::client::legacy::Client::builder(TokioExecutor::new()).build(https);

        let response = client.request(request).await.expect("/calls.json");

        let body = response.collect().await.expect("Response").to_bytes();

        let calls =
            serde_json::from_slice::<HashMap<String, HashMap<String, usize>>>(body.as_ref())
                .expect("Invalid JSON request");

        calls[service][rpc]
    }
}

impl Client {
    /// Build a client from the engine's channel
    pub fn with_channel(channel: GrpcChannel) -> Self {
        Self { channel }
    }

    #[cfg(feature = "worker")]
    /// Create a borrowed [`Agent`]
    pub fn agent(&mut self) -> Agent {
        Agent::with_channel(self.channel.clone())
    }
    #[cfg(feature = "worker")]
    /// Create an owned [`Agent`]
    pub fn into_agent(self) -> Agent {
        Agent::with_channel(self.channel)
    }

    #[cfg(feature = "client")]
    /// Create a borrowed [`Applications`]
    pub fn applications(&mut self) -> Applications {
        Applications::with_channel(self.channel.clone())
    }
    #[cfg(feature = "client")]
    /// Create an owned [`Applications`]
    pub fn into_applications(self) -> Applications {
        Applications::with_channel(self.channel)
    }

    #[cfg(feature = "client")]
    /// Create a borrowed [`Auth`]
    pub fn auth(&mut self) -> Auth {
        Auth::with_channel(self.channel.clone())
    }
    #[cfg(feature = "client")]
    /// Create an owned [`Auth`]
    pub fn into_auth(self) -> Auth {
        Auth::with_channel(self.channel)
    }

    #[cfg(feature = "client")]
    /// Create a borrowed [`Events`]
    pub fn events(&mut self) -> Events {
        Events::with_channel(self.channel.clone())
    }
    #[cfg(feature = "client")]
    /// Create an owned [`Events`]
    pub fn into_events(self) -> Events {
        Events::with_channel(self.channel)
    }

    #[cfg(feature = "client")]
    /// Create a borrowed [`HealthChecks`]
    pub fn health_checks(&mut self) -> HealthChecks {
        HealthChecks::with_channel(self.channel.clone())
    }
    #[cfg(feature = "client")]
    /// Create an owned [`HealthChecks`]
    pub fn into_health_checks(self) -> HealthChecks {
        HealthChecks::with_channel(self.channel)
    }

    #[cfg(feature = "client")]
    /// Create a borrowed [`Partitions`]
    pub fn partitions(&mut self) -> Partitions {
        Partitions::with_channel(self.channel.clone())
    }
    #[cfg(feature = "client")]
    /// Create an owned [`Partitions`]
    pub fn into_partitions(self) -> Partitions {
        Partitions::with_channel(self.channel)
    }

    #[cfg(feature = "client")]
    /// Create a borrowed [`Results`]
    pub fn results(&mut self) -> Results {
        Results::with_channel(self.channel.clone())
    }
    #[cfg(feature = "client")]
    /// Create an owned [`Results`]
    pub fn into_results(self) -> Results {
        Results::with_channel(self.channel)
    }

    #[cfg(feature = "client")]
    /// Create a borrowed [`Sessions`]
    pub fn sessions(&mut self) -> Sessions {
        Sessions::with_channel(self.channel.clone())
    }
    #[cfg(feature = "client")]
    /// Create an owned [`Sessions`]
    pub fn into_sessions(self) -> Sessions {
        Sessions::with_channel(self.channel)
    }

    /// Create a borrowed [`Submitter`]
    #[cfg(feature = "client")]
    #[deprecated]
    #[allow(deprecated)]
    pub fn submitter(&mut self) -> Submitter {
        Submitter::with_channel(self.channel.clone())
    }
    #[cfg(feature = "client")]
    #[deprecated]
    #[allow(deprecated)]
    /// Create an owned [`Submitter`]
    pub fn into_submitter(self) -> Submitter {
        Submitter::with_channel(self.channel)
    }

    #[cfg(feature = "client")]
    /// Create a borrowed [`Tasks`]
    pub fn tasks(&mut self) -> Tasks {
        Tasks::with_channel(self.channel.clone())
    }
    #[cfg(feature = "client")]
    /// Create an owned [`Tasks`]
    pub fn into_tasks(self) -> Tasks {
        Tasks::with_channel(self.channel)
    }

    #[cfg(feature = "client")]
    /// Create a borrowed [`Versions`]
    pub fn versions(&mut self) -> Versions {
        Versions::with_channel(self.channel.clone())
    }
    #[cfg(feature = "client")]
    /// Create an owned [`Versions`]
    pub fn into_versions(self) -> Versions {
        Versions::with_channel(self.channel)
    }

    #[cfg(feature = "agent")]
    /// Create a borrowed [`Worker`]
    pub fn worker(&mut self) -> Worker {
        Worker::with_channel(self.channel.clone())
    }
    #[cfg(feature = "agent")]
    /// Create an owned [`Worker`]
    pub fn into_worker(self) -> Worker {
        Worker::with_channel(self.channel)
    }
}

/// Perform a gRPC call from a raw request.
#[allow(async_fn_in_trait)]
pub trait GrpcCall<Request> {
    type Response;
    type Error;

    /// Perform a gRPC call from a raw request.
    async fn call(self, request: Request) -> Result<Self::Response, Self::Error>;
}

/// Perform a gRPC call from a raw request.
#[allow(async_fn_in_trait)]
pub trait GrpcCallStream<Request, Stream>
where
    Stream: futures::Stream<Item = Request> + Send + 'static,
{
    type Response;
    type Error;

    /// Perform a gRPC call from a raw request.
    async fn call(self, request: Stream) -> Result<Self::Response, Self::Error>;
}

impl<Stream, Request, T> GrpcCall<Stream> for T
where
    Stream: futures::Stream<Item = Request> + Send + 'static,
    T: GrpcCallStream<Request, Stream>,
{
    type Response = <T as GrpcCallStream<Request, Stream>>::Response;
    type Error = <T as GrpcCallStream<Request, Stream>>::Error;

    /// Perform a gRPC call from a raw request.
    async fn call(self, request: Stream) -> Result<Self::Response, Self::Error> {
        <T as GrpcCallStream<Request, Stream>>::call(self, request).await
    }
}

#[derive(Debug, Snafu)]
#[non_exhaustive]
pub enum RequestError {
    #[snafu(display("Grpc request error [{location}]"))]
    #[non_exhaustive]
    Grpc {
        #[snafu(source(from(GrpcStatus, Box::new)))]
        source: Box<GrpcStatus>,
        #[snafu(implicit)]
        location: snafu::Location,
    },
}

macro_rules! impl_call {
    (@one $Client:ident($self:ident, $request:ident: $Request:ty) -> Result<$Response:ty> $block:block) => {
        crate::client::impl_call! {
            @one $Client($self, $request: $Request) -> Result<$Response, crate::client::RequestError> $block
        }
    };
    (@one $Client:ident($self:ident, $request:ident: $Request:ty) -> Result<$Response:ty, $Error:ty> $block:block) => {
        impl $crate::client::GrpcCall<$Request> for &'_ mut $Client {
            type Response = $Response;
            type Error = $Error;

            async fn call($self, $request: $Request) -> Result<Self::Response, Self::Error> $block
        }
    };
    ($Client:ident {$(async fn call($self:ident, $request:ident: $Request:ty) -> Result<$($Result:ty),*> $block:block)*}) => {
        $(
            crate::client::impl_call! {
                @one $Client($self, $request: $Request) -> Result<$($Result),*> $block
            }
        )*
    };
}

pub(crate) use impl_call;

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use armonik_transport::ClientConfig;

    use crate::Client;

    /// What the span records, on the one path that reaches it with a password.
    ///
    /// `ClientConfig::endpoint` is a public field, so a config built here rather than read from
    /// the environment never met the check that refuses userinfo. The connection is expected to
    /// fail - the span is created before the dial and is what this reads.
    #[tokio::test]
    async fn the_client_span_renders_the_endpoint_rather_than_printing_it() {
        #[derive(Clone, Default)]
        struct Captured(Arc<Mutex<Vec<u8>>>);

        impl Write for Captured {
            fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
                self.0.lock().expect("the buffer").extend_from_slice(buffer);
                Ok(buffer.len())
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
            type Writer = Self;

            fn make_writer(&'a self) -> Self::Writer {
                self.clone()
            }
        }

        let captured = Captured::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(captured.clone())
            .with_max_level(tracing::Level::DEBUG)
            .with_span_events(tracing_subscriber::fmt::format::FmtSpan::NEW)
            .finish();

        let mut config = ClientConfig::default();
        config.endpoint = hyper::Uri::try_from("http://alice:s3cret@127.0.0.1:1").expect("a uri");
        config.allow_unsafe_connection = true;

        {
            let _guard = tracing::subscriber::set_default(subscriber);
            let _ = Client::with_config(config).await;
        }

        let said = String::from_utf8(captured.0.lock().expect("the buffer").clone())
            .expect("what the subscriber wrote");

        assert!(
            said.contains("127.0.0.1:1"),
            "the span says nothing: {said}"
        );
        assert!(!said.contains("s3cret"), "the span repeats it: {said}");
        assert!(!said.contains("alice"), "the span repeats it: {said}");
    }
}
