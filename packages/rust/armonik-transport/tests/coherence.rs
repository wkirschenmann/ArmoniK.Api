//! Options that are each valid and cannot hold together, found once the options are merged: a
//! channel's are refused, and a runtime's defaults are said, since a channel can still override
//! them.

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use armonik_transport::configuration::Configuration;
use armonik_transport::options::{
    ChannelOptions, ExponentialBackoffOptions, RetryOptions, Seconds, TcpKeepalive, TcpProbe,
};
use armonik_transport::settings::{ChannelSettings, SettingRefusal};
use tracing::field::{Field, Visit};
use tracing::Level;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::Layer;

/// The level and the message of every event.
#[derive(Clone, Default)]
struct Logged(Arc<Mutex<Vec<(Level, String)>>>);

impl Logged {
    fn warnings(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(level, _)| *level == Level::WARN)
            .map(|(_, said)| said.clone())
            .collect()
    }
}

impl<S: tracing::Subscriber> Layer<S> for Logged {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        struct Message(String);

        impl Visit for Message {
            fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
                if field.name() == "message" {
                    self.0 = format!("{value:?}");
                }
            }
        }

        let mut message = Message(String::new());
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((*event.metadata().level(), message.0));
    }
}

fn options(json: &str) -> ChannelOptions {
    Configuration::with_prefix("")
        .document(json)
        .load()
        .expect("a document")
}

/// What `settle_defaults` logs at the warning level, and whether it refused.
fn defaults(json: &str) -> (Result<(), String>, Vec<String>) {
    let logged = Logged::default();
    let subscriber = tracing_subscriber::registry().with(logged.clone());
    let settled = tracing::subscriber::with_default(subscriber, || {
        ChannelSettings::settle_defaults(options(json))
    });
    (
        settled.map_err(|refused| refused.to_string()),
        logged.warnings(),
    )
}

/// The incoherences a channel's options are refused for.
fn channel(json: &str) -> Result<(), SettingRefusal> {
    ChannelSettings::settle(options(json)).map(drop)
}

const INCOHERENT: [(&str, &[&str]); 1] = [(
    r#"{"Grpc":{"OutboundTraffic":{"Retry":{"ExponentialBackoff":{"InitialBackoffSeconds":500}}}}}"#,
    &[
        "Grpc.OutboundTraffic.Retry.ExponentialBackoff.InitialBackoffSeconds",
        "Grpc.OutboundTraffic.Retry.ExponentialBackoff.MaxBackoffSeconds",
    ],
)];

#[test]
fn a_channels_incoherent_options_are_refused_naming_every_key() {
    for (json, keys) in INCOHERENT {
        let refused = channel(json).expect_err(json);
        let said = refused.to_string();
        assert!(matches!(refused, SettingRefusal::Incoherent(_)), "{said}");
        for key in keys {
            assert!(said.contains(key), "{json}: {said}");
        }
        assert!(said.contains("incoherent"), "{json}: {said}");
    }
}

#[test]
fn a_runtimes_incoherent_defaults_are_said_and_not_refused() {
    for (json, keys) in INCOHERENT {
        let (settled, warnings) = defaults(json);
        assert_eq!(settled, Ok(()), "{json}");
        assert_eq!(warnings.len(), 1, "{json}: {warnings:?}");
        for key in keys {
            assert!(warnings[0].contains(key), "{json}: {}", warnings[0]);
        }
        assert!(warnings[0].contains("incoherent"), "{}", warnings[0]);
    }
}

#[test]
fn coherent_options_are_neither_refused_nor_said() {
    for json in [
        "{}",
        r#"{"Grpc":{"OutboundTraffic":{"Retry":{"ExponentialBackoff":{"InitialBackoffSeconds":1,"MaxBackoffSeconds":1}}}}}"#,
        r#"{"Transport":{"TcpKeepalive":{"Probe":{"IdleSeconds":30,"IntervalSeconds":5,"Retries":3}}}}"#,
    ] {
        channel(json).expect(json);
        assert_eq!(defaults(json), (Ok(()), Vec::new()), "{json}");
    }
}

/// The check is made once the options are merged, so what the defaults state completes what a
/// channel states.
#[test]
fn what_a_merge_completes_is_coherent() {
    let initial = r#"{"Grpc":{"OutboundTraffic":{"Retry":{"ExponentialBackoff":{"InitialBackoffSeconds":500}}}}}"#;
    let defaults = options(
        r#"{"Grpc":{"OutboundTraffic":{"Retry":{"ExponentialBackoff":{"MaxBackoffSeconds":600}}}}}"#,
    );
    let own = options(initial);
    assert!(channel(initial).is_err());
    ChannelSettings::settle(own.over(&defaults)).expect("an initial of 500 under a maximum of 600");

    // And a channel can override what its runtime's defaults say incoherently.
    let incoherent = options(initial);
    let own = options(
        r#"{"Grpc":{"OutboundTraffic":{"Retry":{"ExponentialBackoff":{"MaxBackoffSeconds":900}}}}}"#,
    );
    ChannelSettings::settle(own.over(&incoherent)).expect("the maximum the defaults lack");
    assert!(ChannelSettings::settle(ChannelOptions::default().over(&incoherent)).is_err());
}

/// A value that is wrong by itself is refused, at the runtime's level too: an incoherence is the
/// one thing a channel's own options can mend.
///
/// The loader refuses such a value as it reads it, so these options are built in code, as a Rust
/// caller may build them.
#[test]
fn a_value_that_is_wrong_by_itself_is_refused_at_both_levels() {
    let mut probe = TcpProbe::new(30);
    probe.interval_seconds = Some(0);
    let mut keepalive = ChannelOptions::default();
    keepalive.transport.tcp_keepalive = Some(TcpKeepalive::Probe(probe));

    let mut backoff = ExponentialBackoffOptions::default();
    backoff.initial_backoff_seconds = Some(Seconds(0.0));
    let mut retried = ChannelOptions::default();
    retried.grpc.outbound_traffic.retry = Some(RetryOptions::ExponentialBackoff(backoff));

    for (options, key) in [
        (keepalive, "IntervalSeconds"),
        (retried, "InitialBackoffSeconds"),
    ] {
        let refused = ChannelSettings::settle(options.clone())
            .map(drop)
            .expect_err(key);
        assert!(matches!(refused, SettingRefusal::Option(_)), "{refused}");
        assert!(refused.to_string().contains(key), "{refused}");
        assert!(
            ChannelSettings::settle_defaults(options).is_err(),
            "{key} in the defaults"
        );
    }
}

/// And a document that states one is refused by the loader, by its path, before any level.
#[test]
fn a_document_that_states_a_value_wrong_by_itself_is_refused_by_the_loader() {
    for (json, key) in [
        (
            r#"{"Transport":{"TcpKeepalive":{"Probe":{"IdleSeconds":30,"IntervalSeconds":0}}}}"#,
            "Transport.TcpKeepalive.Probe.IntervalSeconds",
        ),
        (
            r#"{"Grpc":{"OutboundTraffic":{"Retry":{"ExponentialBackoff":{"InitialBackoffSeconds":0}}}}}"#,
            "Grpc.OutboundTraffic.Retry.ExponentialBackoff.InitialBackoffSeconds",
        ),
    ] {
        let refused = Configuration::with_prefix("")
            .document(json)
            .load::<ChannelOptions>()
            .expect_err(json);
        assert_eq!(refused.key(), Some(key), "{refused}");
    }
}
