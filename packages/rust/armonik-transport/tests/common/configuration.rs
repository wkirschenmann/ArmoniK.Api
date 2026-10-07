//! The configuration fixtures, `tests/configuration.json`: sources in, the options they load or
//! the refusal they earn out, and the keys logged as unknown.

use std::path::PathBuf;

use serde_json::Value;

pub struct Fixture {
    pub name: String,
    /// `None` for the default prefix, and `Some("")` for none.
    pub prefix: Option<String>,
    pub files: Vec<(String, String)>,
    pub environment: Vec<(String, String)>,
    pub sources: Vec<Source>,
    pub outcome: Outcome,
    /// The keys the load logs as unknown, which only the loader's own tests read.
    pub unknown: Vec<String>,
}

pub enum Source {
    File(String),
    OptionalFile(String),
    Environment,
    /// Pairs as the typed API takes them, when the fixture writes an object of text.
    Pairs(Vec<(String, String)>),
    /// Pairs as a JSON text, when the fixture writes one that is not such an object.
    PairsJson(String),
    Document(String),
}

pub enum Outcome {
    /// The options loaded, as a JSON document of `RuntimeOptions`.
    Options(Value),
    Refused {
        /// What the refusal names as its source; a file by the end of its path.
        source: String,
        key: Option<String>,
        /// A text the refusal says.
        says: Option<String>,
        /// A value the refusal must not quote.
        never: Option<String>,
    },
}

pub fn fixtures() -> Vec<Fixture> {
    let all: Value =
        serde_json::from_str(include_str!("../configuration.json")).expect("the fixtures are JSON");
    all.as_array()
        .expect("the fixtures are a list")
        .iter()
        .map(fixture)
        .collect()
}

fn text(value: &Value) -> String {
    value.as_str().expect("a text").to_owned()
}

fn texts(value: Option<&Value>) -> Vec<(String, String)> {
    value
        .and_then(Value::as_object)
        .map(|entries| {
            entries
                .iter()
                .map(|(key, value)| (key.clone(), text(value)))
                .collect()
        })
        .unwrap_or_default()
}

fn fixture(value: &Value) -> Fixture {
    let sources = value["sources"]
        .as_array()
        .expect("sources are a list")
        .iter()
        .map(|source| {
            let (kind, value) = source
                .as_object()
                .and_then(|source| source.iter().next())
                .expect("a source is an object of one key");
            match kind.as_str() {
                "file" => Source::File(text(value)),
                "optional_file" => Source::OptionalFile(text(value)),
                "environment" => Source::Environment,
                "pairs" => match value {
                    Value::String(raw) => Source::PairsJson(raw.clone()),
                    _ => Source::Pairs(texts(Some(value))),
                },
                "document" => match value {
                    Value::String(raw) => Source::Document(raw.clone()),
                    _ => Source::Document(value.to_string()),
                },
                other => panic!("`{other}` is no source"),
            }
        })
        .collect();
    let outcome = match (value.get("options"), value.get("refused")) {
        (Some(options), None) => Outcome::Options(options.clone()),
        (None, Some(refused)) => Outcome::Refused {
            source: text(&refused["source"]),
            key: refused.get("key").map(text),
            says: refused.get("says").map(text),
            never: refused.get("never").map(text),
        },
        _ => panic!("a fixture states its options or its refusal"),
    };
    Fixture {
        name: text(&value["name"]),
        prefix: value.get("prefix").map(text),
        files: texts(value.get("files")),
        environment: texts(value.get("environment")),
        sources,
        outcome,
        unknown: value
            .get("unknown")
            .and_then(Value::as_array)
            .map(|keys| keys.iter().map(text).collect())
            .unwrap_or_default(),
    }
}

/// A fixture's files written and its environment set, both undone when it goes.
pub struct Staged {
    directory: PathBuf,
    environment: Vec<String>,
}

impl Staged {
    /// `test` names the directory, so that two test binaries staging at once write apart.
    pub fn new(fixture: &Fixture, test: &str, index: usize) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "armonik-configuration-{test}-{}-{index}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("a scratch directory");
        for (name, content) in &fixture.files {
            std::fs::write(directory.join(name), content).expect("a fixture's file");
        }
        for (name, value) in &fixture.environment {
            std::env::set_var(name, value);
        }
        Self {
            directory,
            environment: fixture
                .environment
                .iter()
                .map(|(name, _)| name.clone())
                .collect(),
        }
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.directory.join(name)
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        for name in &self.environment {
            std::env::remove_var(name);
        }
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
