//! The configuration loader against its fixtures: a source is a document, judged whole.

#[path = "common/configuration.rs"]
mod fixtures;

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use armonik_transport::configuration::{Configuration, SourceName, DEFAULT_PREFIX};
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
    let mut configuration = Configuration::with_prefix(&fixture.prefix);
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
        let loaded = configuration(fixture, &staged).load::<RuntimeOptions>();
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
    }
}

/// Two spellings of one key in pairs are one key, taken from the later and logged by its name.
#[test]
fn a_key_given_twice_is_taken_from_the_later_and_logged() {
    let logged = Logged::default();
    let subscriber = tracing_subscriber::registry().with(logged.clone());
    let loaded: RuntimeOptions = tracing::subscriber::with_default(subscriber, || {
        Configuration::with_prefix("")
            .pairs([
                ("Endpoint".to_owned(), "http://first.test:1".to_owned()),
                ("endpoint".to_owned(), "http://second.test:2".to_owned()),
            ])
            .load()
            .expect("pairs that name a key twice")
    });

    assert_eq!(loaded.endpoint.as_deref(), Some("http://second.test:2"));
    assert_eq!(logged.keys(), ["endpoint"]);
}

/// A channel's own document goes through the same loader, with an empty prefix: its root is
/// judged as any other struct is.
#[test]
fn a_channel_document_refuses_a_key_at_its_root_as_in_a_group() {
    let loaded: ChannelOptions = Configuration::with_prefix("")
        .document(r#"{"Grpc":{"UserAgent":"armonik"}}"#)
        .load()
        .expect("a document that states what the schema declares");
    assert_eq!(loaded.grpc.user_agent.as_deref(), Some("armonik"));

    let refused = Configuration::with_prefix("")
        .document(r#"{"UserAgnt":"typo","Grpc":{"UserAgent":"armonik"}}"#)
        .load::<ChannelOptions>()
        .expect_err("an unknown key at the root is refused");
    assert_eq!(refused.key(), Some("UserAgnt"));

    let refused = Configuration::with_prefix("")
        .document(r#"{"Grpc":{"UserAgnt":"typo","UserAgent":"armonik"}}"#)
        .load::<ChannelOptions>()
        .expect_err("an unknown key in a group is refused");
    assert_eq!(refused.key(), Some("Grpc.UserAgnt"));
}

/// The logging filter is a key of the runtime's options. A host's own sections, which an empty
/// prefix takes with the rest, are refused by their path, the `Logging` section of a host
/// meeting the runtime's own group.
#[test]
fn the_logging_filter_loads_and_a_hosts_sections_are_refused_under_an_empty_prefix() {
    let loaded: RuntimeOptions = Configuration::with_prefix("")
        .document(r#"{"Logging":{"Filter":"h2=debug"}}"#)
        .load()
        .expect("the engine's own Logging group");
    assert_eq!(loaded.logging.filter.as_deref(), Some("h2=debug"));

    let refused = Configuration::with_prefix("")
        .document(r#"{"Logging":{"Filter":"h2=debug"},"Serilog":{"Level":"Debug"}}"#)
        .load::<RuntimeOptions>()
        .expect_err("a host's section is refused");
    assert_eq!(refused.key(), Some("Serilog"));

    let refused = Configuration::with_prefix("")
        .document(r#"{"Logging":{"Filter":"h2=debug","LogLevel":{"Default":"Debug"}}}"#)
        .load::<RuntimeOptions>()
        .expect_err("a host's Logging section is refused where it meets the runtime's");
    assert_eq!(refused.key(), Some("Logging.LogLevel"));
}

/// A host's appsettings.json keeps its own sections and the engine's options under
/// `ArmoniK:Client:Grpc`: read with that prefix it loads, and only that section is looked at.
#[test]
fn a_hosts_appsettings_loads_under_the_prefix_and_is_refused_without_it() {
    let settings = r#"{
        "Logging": { "LogLevel": { "Default": "Information" } },
        "Serilog": { "MinimumLevel": "Debug" },
        "ArmoniK": { "Client": { "Grpc": { "Endpoint": "http://host.test:5001" } } }
    }"#;

    let loaded: RuntimeOptions = Configuration::with_prefix("ArmoniK:Client:Grpc")
        .document(settings)
        .load()
        .expect("the section the prefix names conforms to the schema");
    assert_eq!(loaded.endpoint.as_deref(), Some("http://host.test:5001"));

    let loaded: RuntimeOptions = Configuration::with_prefix(DEFAULT_PREFIX)
        .document(settings)
        .load()
        .expect("the prefix written with `__` names the same section");
    assert_eq!(loaded.endpoint.as_deref(), Some("http://host.test:5001"));

    let refused = Configuration::with_prefix("")
        .document(settings)
        .load::<RuntimeOptions>()
        .expect_err("the whole file is the engine's, and Logging.LogLevel is no option");
    assert_eq!(refused.key(), Some("Logging.LogLevel"));
}
