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

/// Every event the load logs, with its level and fields.
#[derive(Clone, Default)]
struct Logged(Arc<Mutex<Vec<Event>>>);

#[derive(Clone, Debug)]
struct Event {
    level: tracing::Level,
    target: String,
    fields: Vec<(String, String)>,
}

impl Event {
    fn field(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.as_str())
    }
}

impl Logged {
    fn events(&self) -> Vec<Event> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The `key` of every event that has one.
    fn keys(&self) -> Vec<String> {
        self.events()
            .iter()
            .filter_map(|event| event.field("key").map(str::to_owned))
            .collect()
    }
}

impl<S: tracing::Subscriber> Layer<S> for Logged {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        struct Fields(Vec<(String, String)>);

        impl Visit for Fields {
            fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
                self.0.push((field.name().to_owned(), format!("{value:?}")));
            }
        }

        let mut fields = Fields(Vec::new());
        event.record(&mut fields);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Event {
                level: *event.metadata().level(),
                target: event.metadata().target().to_owned(),
                fields: fields.0,
            });
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

/// What a load of `configuration` gives and logs.
fn load_logged(configuration: &Configuration) -> (RuntimeOptions, Vec<Event>) {
    let logged = Logged::default();
    let subscriber = tracing_subscriber::registry().with(logged.clone());
    let loaded = tracing::subscriber::with_default(subscriber, || {
        configuration.load().expect("a source that loads")
    });
    (loaded, logged.events())
}

/// A scratch directory, removed when it goes, a failed test included.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "armonik-configuration-{name}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("a scratch directory");
        Self(directory)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const PREFIX: &str = "ArmoniK:Client:Grpc";

/// The prefix as the warning writes it: its parts joined by `__`.
const STORED: &str = "ArmoniK__Client__Grpc";

/// A document with no section under the prefix loads as the defaults do, and says so.
#[test]
fn a_document_without_the_prefixs_section_loads_the_defaults_and_warns() {
    let (loaded, logged) = load_logged(
        &Configuration::with_prefix(PREFIX)
            .document(r#"{"Logging":{"LogLevel":{"Default":"Information"}}}"#),
    );

    assert_eq!(loaded, RuntimeOptions::default());
    assert_eq!(logged.len(), 1, "{logged:#?}");
    assert_eq!(logged[0].level, tracing::Level::WARN);
    assert!(logged[0].target.starts_with("armonik_transport"));
    assert_eq!(logged[0].field("source"), Some("a document"));
    assert_eq!(logged[0].field("prefix"), Some(STORED));
    assert_eq!(logged[0].field("section"), None);
}

/// A file with no section under the prefix warns by its path, and a source after it still applies.
#[test]
fn a_file_without_the_prefixs_section_warns_by_its_path_and_the_next_source_applies() {
    let scratch = Scratch::new("absent-section");
    let path = scratch.0.join("appsettings.json");
    std::fs::write(
        &path,
        r#"{"ArmoniK":{"Client":{"Grcp":{"Endpoint":"http://typo.test:1"}}}}"#,
    )
    .expect("a file");

    let (loaded, logged) = load_logged(
        &Configuration::with_prefix(PREFIX)
            .file(&path)
            .document(r#"{"ArmoniK":{"Client":{"Grpc":{"Endpoint":"http://later.test:2"}}}}"#),
    );

    assert_eq!(loaded.endpoint.as_deref(), Some("http://later.test:2"));
    assert_eq!(logged.len(), 1, "{logged:#?}");
    assert_eq!(logged[0].level, tracing::Level::WARN);
    assert_eq!(
        logged[0].field("source"),
        Some(path.display().to_string().as_str())
    );
    assert_eq!(logged[0].field("prefix"), Some(STORED));
    assert_eq!(logged[0].field("section"), None);
}

/// A section that differs from the prefix only by case is not read, and the warning names it.
#[test]
fn a_section_that_differs_by_case_is_named_by_the_warning_and_not_read() {
    for (written, document) in [
        (
            "armonik__client__grpc",
            r#"{"armonik":{"client":{"grpc":{"Endpoint":"http://case.test:1"}}}}"#,
        ),
        (
            "ArmoniK__client__Grpc",
            r#"{"ArmoniK":{"client":{"Grpc":{"Endpoint":"http://case.test:1"}}}}"#,
        ),
        (
            "armonik__Client__Grpc",
            r#"{"ArmoniK":{"Other":{}},"armonik":{"Client":{"Grpc":{"Endpoint":"http://case.test:1"}}}}"#,
        ),
    ] {
        let (loaded, logged) = load_logged(&Configuration::with_prefix(PREFIX).document(document));

        assert_eq!(loaded, RuntimeOptions::default(), "{written}");
        assert_eq!(logged.len(), 1, "{written}: {logged:#?}");
        assert_eq!(logged[0].level, tracing::Level::WARN);
        assert_eq!(logged[0].field("source"), Some("a document"));
        assert_eq!(logged[0].field("prefix"), Some(STORED));
        assert_eq!(logged[0].field("section"), Some(written), "{written}");
    }
}

/// A section the prefix names logs nothing, and nor does an empty prefix, a missing optional file
/// or a prefix that matches no pair.
#[test]
fn a_load_that_finds_its_section_or_reads_names_logs_no_warning() {
    let (loaded, logged) = load_logged(
        &Configuration::with_prefix(PREFIX)
            .document(r#"{"ArmoniK":{"Client":{"Grpc":{"Endpoint":"http://host.test:1"}}}}"#),
    );
    assert_eq!(loaded.endpoint.as_deref(), Some("http://host.test:1"));
    assert!(logged.is_empty(), "{logged:#?}");

    let (_, logged) = load_logged(&Configuration::with_prefix("").document("{}"));
    assert!(logged.is_empty(), "{logged:#?}");

    let scratch = Scratch::new("optional-file");
    let (loaded, logged) = load_logged(
        &Configuration::with_prefix(PREFIX).optional_file(scratch.0.join("missing.json")),
    );
    assert_eq!(loaded, RuntimeOptions::default());
    assert!(logged.is_empty(), "{logged:#?}");

    let (loaded, logged) = load_logged(
        &Configuration::with_prefix(PREFIX).pairs([("Other__Endpoint".to_owned(), "x".to_owned())]),
    );
    assert_eq!(loaded, RuntimeOptions::default());
    assert!(logged.is_empty(), "{logged:#?}");
}
