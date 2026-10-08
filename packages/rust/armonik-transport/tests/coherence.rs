//! Options that are each valid and cannot hold together, found once the options are merged: a
//! channel's are refused, and a runtime's defaults are said, since a channel can still override
//! them.

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use armonik_transport::configuration::Configuration;
use armonik_transport::options::ChannelOptions;
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

const INCOHERENT: [(&str, &[&str]); 5] = [
    (
        r#"{"Grpc":{"Retry":{"InitialBackoffSeconds":10}}}"#,
        &[
            "Grpc.Retry.InitialBackoffSeconds",
            "Grpc.Retry.MaxBackoffSeconds",
        ],
    ),
    (
        r#"{"Grpc":{"Rate":{"Limit":{"Calls":5}}}}"#,
        &["Grpc.Rate.Limit.Calls", "Grpc.Rate.Limit.PerSeconds"],
    ),
    (
        r#"{"Grpc":{"Rate":{"Limit":{"PerSeconds":1}}}}"#,
        &["Grpc.Rate.Limit.Calls", "Grpc.Rate.Limit.PerSeconds"],
    ),
    (
        r#"{"Transport":{"TcpKeepalive":{"IntervalSeconds":5}}}"#,
        &[
            "Transport.TcpKeepalive.IdleSeconds",
            "Transport.TcpKeepalive.IntervalSeconds",
        ],
    ),
    (
        r#"{"Transport":{"TcpKeepalive":{"IntervalSeconds":5,"Retries":3}}}"#,
        &[
            "Transport.TcpKeepalive.IdleSeconds",
            "Transport.TcpKeepalive.IntervalSeconds",
            "Transport.TcpKeepalive.Retries",
        ],
    ),
];

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
        r#"{"Grpc":{"Retry":{"InitialBackoffSeconds":1,"MaxBackoffSeconds":1}}}"#,
        r#"{"Grpc":{"Rate":{"Limit":{"Calls":5,"PerSeconds":1}}}}"#,
        r#"{"Transport":{"TcpKeepalive":{"IdleSeconds":30,"IntervalSeconds":5,"Retries":3}}}"#,
    ] {
        channel(json).expect(json);
        assert_eq!(defaults(json), (Ok(()), Vec::new()), "{json}");
    }
}

/// The check is made once the options are merged, so what the defaults state completes what a
/// channel states.
#[test]
fn what_a_merge_completes_is_coherent() {
    let defaults = options(r#"{"Grpc":{"Retry":{"MaxBackoffSeconds":60}}}"#);
    let own = options(r#"{"Grpc":{"Retry":{"InitialBackoffSeconds":10}}}"#);
    assert!(channel(r#"{"Grpc":{"Retry":{"InitialBackoffSeconds":10}}}"#).is_err());
    ChannelSettings::settle(own.over(&defaults)).expect("an initial of 10 under a maximum of 60");

    // And a channel can override what its runtime's defaults say incoherently.
    let incoherent = options(r#"{"Grpc":{"Rate":{"Limit":{"Calls":5}}}}"#);
    let own = options(r#"{"Grpc":{"Rate":{"Limit":{"PerSeconds":2}}}}"#);
    ChannelSettings::settle(own.over(&incoherent)).expect("the window the defaults lack");
    assert!(ChannelSettings::settle(ChannelOptions::default().over(&incoherent)).is_err());
}

/// A zero turns a feature off, and the options of it that an earlier source left are unread
/// rather than incoherent; a value that is wrong by itself is still wrong.
#[test]
fn what_a_zero_turns_off_is_unread_and_not_incoherent() {
    for json in [
        r#"{"Transport":{"TcpKeepalive":{"IdleSeconds":0,"IntervalSeconds":5,"Retries":3}}}"#,
        r#"{"Grpc":{"Rate":{"Limit":{"Calls":0,"PerSeconds":2}}}}"#,
        r#"{"Grpc":{"Rate":{"Limit":{"Calls":0}}}}"#,
    ] {
        channel(json).expect(json);
        assert_eq!(defaults(json), (Ok(()), Vec::new()), "{json}");
    }

    for (json, key) in [
        (
            r#"{"Transport":{"TcpKeepalive":{"IdleSeconds":0,"IntervalSeconds":0}}}"#,
            "IntervalSeconds",
        ),
        (
            r#"{"Grpc":{"Rate":{"Limit":{"Calls":0,"PerSeconds":0}}}}"#,
            "PerSeconds",
        ),
    ] {
        let refused = channel(json).expect_err(json);
        assert!(matches!(refused, SettingRefusal::Option(_)), "{refused}");
        assert!(refused.to_string().contains(key), "{refused}");
        // A value wrong by itself is refused at the runtime level too: an incoherence is the one
        // thing a channel's own options can mend.
        assert!(defaults(json).0.is_err(), "{json}");
    }
}
