//! The configuration loader against its fixtures, and the keys it logs as unknown.

#[path = "common/configuration.rs"]
mod fixtures;

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use armonik_transport::configuration::{ConfigRefusal, Configuration, SourceName};
use armonik_transport::options::{ChannelOptions, RuntimeOptions};
use fixtures::{Fixture, Outcome, Source, Staged};
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::Layer;

/// The `key` of every event the load logs.
#[derive(Clone, Default)]
struct Logged(Arc<Mutex<Vec<String>>>);

impl Logged {
    fn keys(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl<S: tracing::Subscriber> Layer<S> for Logged {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        struct Key(Option<String>);

        impl Visit for Key {
            fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
                if field.name() == "key" {
                    self.0 = Some(format!("{value:?}"));
                }
            }
        }

        let mut key = Key(None);
        event.record(&mut key);
        if let Some(key) = key.0 {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(key);
        }
    }
}

fn configuration(fixture: &Fixture, staged: &Staged) -> Configuration {
    let mut configuration = match &fixture.prefix {
        None => Configuration::new(),
        Some(prefix) => Configuration::with_prefix(prefix),
    };
    for source in &fixture.sources {
        configuration = match source {
            Source::File(name) => configuration.file(staged.path(name)),
            Source::OptionalFile(name) => configuration.optional_file(staged.path(name)),
            Source::Environment => configuration.environment(),
            Source::Pairs(pairs) => configuration.pairs(pairs.clone()),
            Source::PairsJson(json) => configuration.pairs_json(json.clone()),
            Source::Document(json) => configuration.document(json.clone()),
        };
    }
    configuration
}

/// Every fixture, one after the other: they set the process's environment.
#[test]
fn every_fixture_loads_the_options_or_the_refusal_it_states() {
    for (index, fixture) in fixtures::fixtures().iter().enumerate() {
        let staged = Staged::new(fixture, "loader", index);
        let logged = Logged::default();
        let subscriber = tracing_subscriber::registry().with(logged.clone());
        let loaded = tracing::subscriber::with_default(subscriber, || {
            configuration(fixture, &staged).load::<RuntimeOptions>()
        });
        let name = &fixture.name;

        match (&fixture.outcome, loaded) {
            (Outcome::Options(expected), Ok(loaded)) => {
                let expected: RuntimeOptions =
                    serde_json::from_value(expected.clone()).expect("the fixture's options");
                assert_eq!(loaded, expected, "{name}");
            }
            (
                Outcome::Refused {
                    source,
                    key,
                    says,
                    never,
                },
                Err(refused),
            ) => {
                let said = refused.to_string();
                match refused.source_name() {
                    SourceName::File(path) => assert!(path.ends_with(source), "{name}: {said}"),
                    named => assert_eq!(&named.to_string(), source, "{name}: {said}"),
                }
                assert_eq!(refused.key(), key.as_deref(), "{name}: {said}");
                if let Some(says) = says {
                    assert!(said.contains(says.as_str()), "{name}: {said}");
                }
                if let Some(never) = never {
                    assert!(!said.contains(never.as_str()), "{name}: {said}");
                }
            }
            (Outcome::Options(_), Err(refused)) => panic!("{name} is refused: {refused}"),
            (Outcome::Refused { .. }, Ok(loaded)) => panic!("{name} loads {loaded:?}"),
        }
        assert_eq!(logged.keys(), fixture.unknown, "{name}");
    }
}

/// A channel's own document goes through the same loader: its root logs what it does not
/// declare, and a group of it refuses one.
#[test]
fn a_channel_document_logs_what_its_root_does_not_declare_and_refuses_the_rest() {
    let logged = Logged::default();
    let subscriber = tracing_subscriber::registry().with(logged.clone());
    let loaded: Result<ChannelOptions, ConfigRefusal> =
        tracing::subscriber::with_default(subscriber, || {
            Configuration::with_prefix("")
                .document(r#"{"UserAgnt":"typo","Grpc":{"UserAgent":"armonik"}}"#)
                .load()
        });

    let loaded = loaded.expect("an unknown key at the root is no refusal");
    assert_eq!(loaded.grpc.user_agent.as_deref(), Some("armonik"));
    assert_eq!(logged.keys(), ["UserAgnt"]);

    let refused = Configuration::with_prefix("")
        .document(r#"{"Grpc":{"UserAgnt":"typo","UserAgent":"armonik"}}"#)
        .load::<ChannelOptions>()
        .expect_err("an unknown key in a group is refused");
    assert_eq!(refused.key(), Some("Grpc.UserAgnt"));
}

/// The logging filter is a key of the runtime's options. A host's own section, read with no
/// prefix, is logged at the root, and its `Logging` section, which meets the runtime's own group,
/// is refused by its path.
#[test]
fn the_logging_filter_loads_and_a_hosts_sections_are_logged_or_refused() {
    let logged = Logged::default();
    let subscriber = tracing_subscriber::registry().with(logged.clone());
    let loaded: Result<RuntimeOptions, ConfigRefusal> =
        tracing::subscriber::with_default(subscriber, || {
            Configuration::with_prefix("")
                .document(r#"{"Logging":{"Filter":"h2=debug"},"Serilog":{"Level":"Debug"}}"#)
                .load()
        });

    let loaded = loaded.expect("an unknown key at the root is no refusal");
    assert_eq!(loaded.logging.filter.as_deref(), Some("h2=debug"));
    assert_eq!(logged.keys(), ["Serilog"]);

    let refused = Configuration::with_prefix("")
        .document(r#"{"Logging":{"Filter":"h2=debug","LogLevel":{"Default":"Debug"}}}"#)
        .load::<RuntimeOptions>()
        .expect_err("a host's Logging section is refused where it meets the runtime's");
    assert_eq!(refused.key(), Some("Logging.LogLevel"));
}
