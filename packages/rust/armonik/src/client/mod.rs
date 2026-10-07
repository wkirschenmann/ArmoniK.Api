//! ArmoniK clients for all the services

use snafu::{ResultExt, Snafu};

// Re-exported here, so a caller reaches them through the client rather than through the transport
// crate.
#[cfg(feature = "_gen-client")]
pub use armonik_transport::configuration::{ConfigRefusal, Configuration};
#[cfg(feature = "_gen-client")]
pub use armonik_transport::grpc::{GrpcChannel, GrpcStatus, GrpcStatusCode};
#[cfg(feature = "_gen-client")]
pub use armonik_transport::options::{self, RuntimeOptions};
#[cfg(feature = "_gen-client")]
pub use armonik_transport::settings::SettingRefusal;

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
    Config { source: ConfigRefusal },
    #[snafu(display("the client's configuration names no Endpoint"))]
    #[non_exhaustive]
    NoEndpoint {},
    // The endpoint is not quoted: a URI may carry credentials in its userinfo.
    #[snafu(display("the client's Endpoint is not a URI such as http://host:port"))]
    #[non_exhaustive]
    Endpoint {},
    #[snafu(display("the client's channel defaults are refused"))]
    #[non_exhaustive]
    Options { source: SettingRefusal },
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
    /// Create a new client configured by the environment: the `ArmoniK__Client__Grpc__*`
    /// variables, read as [`Configuration::environment`] reads them. `ArmoniK__Client__Grpc__Endpoint`
    /// is required.
    pub async fn new() -> Result<Self, ConnectionError> {
        Self::with_configuration(&Configuration::new().environment()).await
    }

    /// Create a new client configured by the sources `configuration` lists, connected once it
    /// returns.
    pub async fn with_configuration(
        configuration: &Configuration,
    ) -> Result<Self, ConnectionError> {
        Self::with_options(configuration.load().context(ConfigSnafu {})?).await
    }

    /// Create a new client to the options' `Endpoint`, its channel configured by their
    /// `ChannelDefaults`, connected once it returns whatever `Transport.ConnectEagerly` says. The
    /// memory ceilings and `Grpc.Host.Receive.Window`, which bound what a host binding holds, have
    /// no effect here; the window is still refused outside the bounds the schema gives it.
    ///
    /// The client's connections run on the tokio runtime current here, which must outlive it.
    pub async fn with_options(options: RuntimeOptions) -> Result<Self, ConnectionError> {
        let endpoint: armonik_transport::reexports::http::Uri = options
            .endpoint
            .as_deref()
            .ok_or(NoEndpointSnafu {}.build())?
            .parse()
            .map_err(|_| EndpointSnafu {}.build())?;
        let settings = armonik_transport::settings::ChannelSettings::settle(
            options.channel_defaults.unwrap_or_default(),
        )
        .context(OptionsSnafu {})?;

        // Rendered rather than printed: the endpoint may carry `user:password@`, which the
        // channel refuses after this span is made, and the span is a log line.
        let span_endpoint = armonik_transport::safe_endpoint(&endpoint);
        tracing_futures::Instrument::instrument(
            async move {
                let channel = GrpcChannel::new(
                    settings.into_channel_config(endpoint),
                    tokio::runtime::Handle::current(),
                )
                .context(ChannelSnafu {})?;
                channel.connect().await.context(ConnectSnafu {})?;
                Ok(Self::with_channel(channel))
            },
            tracing::debug_span!("Client", endpoint = span_endpoint),
        )
        .await
    }

    #[cfg(test)]
    async fn get_nb_request(service: &str, rpc: &str) -> usize {
        use std::collections::HashMap;

        use http_body_util::BodyExt;
        use hyper_util::rt::TokioExecutor;

        let options: RuntimeOptions = Configuration::new()
            .environment()
            .load()
            .expect("the environment's configuration");
        let mut endpoint = options.endpoint.expect("an Endpoint");

        match std::env::var("Http__Endpoint") {
            Ok(value) if !value.is_empty() => endpoint = value,
            Ok(_) | Err(std::env::VarError::NotPresent) => {}
            Err(std::env::VarError::NotUnicode(value)) => {
                panic!("{value:?} is not a valid unicode string")
            }
        }
        let endpoint: hyper::Uri = endpoint.parse().expect("HTTP endpoint");

        let request = hyper::Request::get(format!("{endpoint}calls.json"))
            .body(http_body_util::Empty::<&[u8]>::new())
            .expect("Request");

        let transport = armonik_transport::settings::ChannelSettings::settle(
            options.channel_defaults.unwrap_or_default(),
        )
        .expect("the channel defaults")
        .into_channel_config(endpoint)
        .transport;
        let https = armonik_transport::https_connector(transport)
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

    use super::{options::ChannelOptions, Configuration, ConnectionError, RuntimeOptions};
    use crate::Client;

    #[tokio::test]
    async fn a_client_with_no_endpoint_is_refused() {
        let refused = Client::with_configuration(&Configuration::new().document("{}"))
            .await
            .err();
        assert!(
            matches!(refused, Some(ConnectionError::NoEndpoint { .. })),
            "{refused:?}"
        );
    }

    #[tokio::test]
    async fn a_client_whose_configuration_is_refused_is_refused() {
        let refused =
            Client::with_configuration(&Configuration::new().document(r#"{"Endpoint": 1}"#))
                .await
                .err();
        assert!(
            matches!(refused, Some(ConnectionError::Config { .. })),
            "{refused:?}"
        );
    }

    #[tokio::test]
    async fn a_client_whose_endpoint_is_no_uri_is_refused() {
        let mut options = RuntimeOptions::default();
        options.endpoint = Some("http://[".to_owned());
        let refused = Client::with_options(options).await.err();
        assert!(
            matches!(refused, Some(ConnectionError::Endpoint { .. })),
            "{refused:?}"
        );
    }

    #[tokio::test]
    async fn a_client_whose_channel_defaults_are_refused_is_refused() {
        let mut defaults = ChannelOptions::default();
        defaults.grpc.host.receive.window = Some(0);
        let mut options = RuntimeOptions::default();
        options.endpoint = Some("http://127.0.0.1:1".to_owned());
        options.channel_defaults = Some(defaults);
        let refused = Client::with_options(options).await.err();
        assert!(
            matches!(refused, Some(ConnectionError::Options { .. })),
            "{refused:?}"
        );
    }

    /// What the span records, on the one path that reaches it with a password.
    ///
    /// The span is made before the channel refuses the endpoint's userinfo, so it is what this
    /// reads; the client is refused.
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

        let mut options = RuntimeOptions::default();
        options.endpoint = Some("http://alice:s3cret@127.0.0.1:1".to_owned());

        {
            let _guard = tracing::subscriber::set_default(subscriber);
            let _ = Client::with_options(options).await;
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
