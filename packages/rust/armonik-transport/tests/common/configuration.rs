//! The configuration fixtures, `tests/configuration.json`: sources in, and the options they load or
//! the refusal they earn out.

use std::path::PathBuf;

use armonik_transport::configuration::DEFAULT_PREFIX;
use serde_json::{json, Value};

pub struct Fixture {
    pub name: String,
    /// The prefix the fixture loads under, `ArmoniK__Client__Grpc` unless it names another, and
    /// empty to take everything.
    pub prefix: String,
    pub files: Vec<(String, String)>,
    pub environment: Vec<(String, String)>,
    pub sources: Vec<Source>,
    pub outcome: Outcome,
}

pub enum Source {
    File(String),
    OptionalFile(String),
    Environment,
    /// Pairs as the typed API takes them, when the fixture writes an object of text: each name
    /// is the key's path under the fixture's prefix.
    Pairs(Vec<(String, String)>),
    /// Pairs as a JSON text, when the fixture writes one that is not such an object.
    PairsJson(String),
    /// A document, which the fixture writes as an object of the engine's options, put under the
    /// fixture's prefix here, or as a text, which is taken as it is.
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

/// The parts of a prefix, as the loader reads one: joined by `__`, or by `:`.
fn parts(prefix: &str) -> Vec<String> {
    if prefix.is_empty() {
        return Vec::new();
    }
    prefix
        .replace(':', "__")
        .split("__")
        .map(str::to_owned)
        .collect()
}

/// `options` as the section a prefix names, nested as a file nests it.
fn nested(prefix: &str, options: Value) -> Value {
    parts(prefix)
        .into_iter()
        .rev()
        .fold(options, |inner, part| json!({ part: inner }))
}

/// A key's path under the prefix, as the name of a variable or of a pair.
fn named(prefix: &str, path: &str) -> String {
    let mut names = parts(prefix);
    names.push(path.to_owned());
    names.join("__")
}

fn fixture(value: &Value) -> Fixture {
    let prefix = value
        .get("prefix")
        .map_or_else(|| DEFAULT_PREFIX.to_owned(), text);
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
                    _ => Source::Pairs(
                        texts(Some(value))
                            .into_iter()
                            .map(|(path, text)| (named(&prefix, &path), text))
                            .collect(),
                    ),
                },
                "document" => match value {
                    Value::String(raw) => Source::Document(raw.clone()),
                    _ => Source::Document(nested(&prefix, value.clone()).to_string()),
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
        prefix,
        files: texts(value.get("files")),
        environment: texts(value.get("environment")),
        sources,
        outcome,
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
