//! The loader and the committed schemas agree: every fixture of `configuration.json` is judged by a
//! JSON Schema validator and by the loader, and each accepts and refuses the same sources.
//!
//! A source is a document, and the schema judges documents. A file or a document is one as it
//! stands, its section the prefix names. The environment and pairs hold text, so each is built
//! into a document here, by the schema: the path from the name, and each value by the type the
//! schema gives its key. What a document cannot hold is refused at that step, as the loader
//! refuses it: a key of a file or a document stated twice, a key of the environment that holds a
//! value and keys under it, a list that is not a JSON array. A text that is not its key's type
//! stays a text, which the schema refuses.
//!
//! The schema states `format: int32` and `int64` for the sizes the loader reads as integers of
//! that width, and a validator takes a format for an annotation, so the bounds the format names are
//! put in the schema before it judges, as OpenAPI defines the words. The test therefore does not
//! show that the committed schema writes those bounds as numbers.

// What a refusal says is the loader's own test's to read; this one reads its key.
#[allow(dead_code)]
#[path = "common/configuration.rs"]
mod fixtures;

use std::fmt;

use armonik_transport::configuration::Configuration;
use armonik_transport::options::{ChannelOptions, RuntimeOptions};
use fixtures::{Fixture, Outcome, Source, Staged};
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{json, Map, Number, Value};

const RUNTIME_SCHEMA: &str = include_str!("../runtime.schema.json");
const CHANNEL_SCHEMA: &str = include_str!("../options.schema.json");

/// A JSON text read with a key stated twice refused, which a plain read collapses to the last: the
/// schema cannot see it, the loader refuses it.
struct Strict(Value);

impl<'de> Deserialize<'de> for Strict {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Any;

        impl<'de> Visitor<'de> for Any {
            type Value = Value;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a JSON value")
            }

            fn visit_unit<E>(self) -> Result<Value, E> {
                Ok(Value::Null)
            }

            fn visit_bool<E>(self, value: bool) -> Result<Value, E> {
                Ok(Value::Bool(value))
            }

            fn visit_u64<E>(self, value: u64) -> Result<Value, E> {
                Ok(json!(value))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Value, E> {
                Ok(json!(value))
            }

            fn visit_f64<E>(self, value: f64) -> Result<Value, E> {
                Ok(json!(value))
            }

            fn visit_str<E>(self, value: &str) -> Result<Value, E> {
                Ok(Value::String(value.to_owned()))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut items: A) -> Result<Value, A::Error> {
                let mut read = Vec::new();
                while let Some(Strict(item)) = items.next_element()? {
                    read.push(item);
                }
                Ok(Value::Array(read))
            }

            fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> Result<Value, A::Error> {
                let mut read = Map::new();
                while let Some(key) = entries.next_key::<String>()? {
                    if read.contains_key(&key) {
                        return Err(de::Error::custom(format_args!("{key} is stated twice")));
                    }
                    let Strict(value) = entries.next_value()?;
                    read.insert(key, value);
                }
                Ok(Value::Object(read))
            }
        }

        deserializer.deserialize_any(Any).map(Strict)
    }
}

fn parse_json(text: &str) -> Result<Value, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    serde_json::from_str::<Strict>(text)
        .map(|strict| strict.0)
        .map_err(|error| error.to_string())
}

fn from_yaml(value: &yaml_rust2::Yaml) -> Result<Value, String> {
    use yaml_rust2::Yaml;
    Ok(match value {
        Yaml::Boolean(value) => Value::Bool(*value),
        Yaml::Integer(value) => json!(value),
        Yaml::Real(text) => text
            .parse::<f64>()
            .map_or_else(|_| Value::String(text.clone()), |number| json!(number)),
        Yaml::String(text) => Value::String(text.clone()),
        Yaml::Array(items) => {
            Value::Array(items.iter().map(from_yaml).collect::<Result<Vec<_>, _>>()?)
        }
        Yaml::Hash(entries) => Value::Object(
            entries
                .iter()
                .map(|(key, value)| {
                    let key = match key {
                        Yaml::String(text) | Yaml::Real(text) => text.clone(),
                        Yaml::Integer(number) => number.to_string(),
                        Yaml::Boolean(flag) => flag.to_string(),
                        _ => return Err("a YAML key that is not a scalar".to_owned()),
                    };
                    Ok((key, from_yaml(value)?))
                })
                .collect::<Result<Map<_, _>, String>>()?,
        ),
        _ => Value::Null,
    })
}

/// A file's content as a document, by its extension.
fn parse_file(name: &str, text: &str) -> Result<Value, String> {
    let extension = name.rsplit('.').next().unwrap_or_default();
    match extension {
        "json" => parse_json(text),
        "yaml" | "yml" => {
            let documents =
                yaml_rust2::YamlLoader::load_from_str(text).map_err(|error| error.to_string())?;
            if documents.len() > 1 {
                return Err("more than one YAML document".to_owned());
            }
            documents.first().map_or_else(|| Ok(json!({})), from_yaml)
        }
        "toml" => toml::from_str::<Value>(text).map_err(|error| error.to_string()),
        _ => Err("an extension that is none of the formats".to_owned()),
    }
}

/// The section a prefix names in a document, nothing when it is missing, and a refusal when what
/// lies on the way is not an object.
fn section(root: Value, prefix: &str) -> Result<Option<Value>, String> {
    if !root.is_object() {
        return Err("not an object".to_owned());
    }
    let mut current = root;
    if prefix.is_empty() {
        return Ok(Some(current));
    }
    for part in prefix.replace(':', "__").split("__") {
        let Value::Object(mut entries) = current else {
            return Err("a section that is not an object".to_owned());
        };
        match entries.remove(part) {
            Some(found) => current = found,
            None => return Ok(None),
        }
    }
    if current.is_object() {
        Ok(Some(current))
    } else {
        Err("the prefix's section is not an object".to_owned())
    }
}

/// The text of the environment or of pairs, by key path.
enum Texts {
    Text(String),
    Map(Vec<(String, Texts)>),
    Both,
}

fn texts_of(values: Vec<(String, String)>) -> Texts {
    fn place(entries: &mut Vec<(String, Texts)>, parts: &[&str], value: String) {
        let Some((first, rest)) = parts.split_first() else {
            return;
        };
        let at = entries
            .iter()
            .position(|(key, _)| key.eq_ignore_ascii_case(first))
            .unwrap_or_else(|| {
                entries.push(((*first).to_owned(), Texts::Map(Vec::new())));
                entries.len() - 1
            });
        let node = &mut entries[at].1;
        if rest.is_empty() {
            *node = match node {
                Texts::Map(children) if children.is_empty() => Texts::Text(value),
                Texts::Map(_) | Texts::Both => Texts::Both,
                Texts::Text(_) => Texts::Text(value),
            };
            return;
        }
        match node {
            Texts::Map(children) => place(children, rest, value),
            _ => *node = Texts::Both,
        }
    }

    let mut root = Vec::new();
    for (path, value) in values {
        let parts: Vec<&str> = path.split("__").collect();
        place(&mut root, &parts, value);
    }
    Texts::Map(root)
}

/// The schema's own reading of a node: its `$ref` followed.
struct Schema(Value);

impl Schema {
    fn resolve<'a>(&'a self, mut node: &'a Value) -> &'a Value {
        while let Some(reference) = node.get("$ref").and_then(Value::as_str) {
            let name = reference.rsplit('/').next().expect("a reference");
            node = &self.0["$defs"][name];
        }
        node
    }

    /// Text built into the document its schema node says it states, or why it states none.
    fn coerce(&self, node: &Value, texts: &Texts, json_lists: bool) -> Result<Value, String> {
        let target = self.resolve(node);
        if matches!(texts, Texts::Both) {
            return Err("a value and keys under it".to_owned());
        }
        if let Some(alternatives) = target.get("oneOf").and_then(Value::as_array) {
            return self.alternative(alternatives, texts, json_lists);
        }
        match (target.get("type").and_then(Value::as_str), texts) {
            (Some("object"), Texts::Map(entries)) => {
                let properties = target.get("properties");
                let mut object = Map::new();
                for (key, child) in entries {
                    let declared = properties
                        .and_then(Value::as_object)
                        .and_then(|properties| {
                            properties
                                .iter()
                                .find(|(name, _)| name.eq_ignore_ascii_case(key))
                        });
                    match declared {
                        Some((name, schema)) => {
                            object.insert(name.clone(), self.coerce(schema, child, json_lists)?);
                        }
                        None => {
                            object.insert(key.clone(), raw(child));
                        }
                    }
                }
                Ok(Value::Object(object))
            }
            (Some("array"), Texts::Text(text)) if json_lists => {
                let Ok(Value::Array(items)) = serde_json::from_str::<Value>(text) else {
                    return Err("a list that is not a JSON array".to_owned());
                };
                let element = target.get("items").expect("a list's items");
                Ok(Value::Array(
                    items
                        .iter()
                        .map(|item| self.coerce_value(element, item))
                        .collect(),
                ))
            }
            (Some("array"), _) => Err("a list that is not stated as a JSON array".to_owned()),
            // A text that is not its key's type stays a text, which the schema refuses. A signed
            // zero, `-0`, is read as the schema reads it and not as an unsigned key does; no
            // fixture states one.
            (Some("integer"), Texts::Text(text)) => Ok(text
                .trim()
                .parse::<i128>()
                .ok()
                .and_then(|number| serde_json::from_str(&number.to_string()).ok())
                .unwrap_or_else(|| Value::String(text.clone()))),
            (Some("number"), Texts::Text(text)) => Ok(text
                .trim()
                .parse::<f64>()
                .ok()
                .and_then(Number::from_f64)
                .map_or_else(|| Value::String(text.clone()), Value::Number)),
            (Some("boolean"), Texts::Text(text)) => {
                Ok(match text.trim().to_ascii_lowercase().as_str() {
                    "true" => Value::Bool(true),
                    "false" => Value::Bool(false),
                    _ => Value::String(text.clone()),
                })
            }
            _ => Ok(raw(texts)),
        }
    }

    /// An alternative: a name, which an environment writes in any case, or an object of one key.
    fn alternative(
        &self,
        alternatives: &[Value],
        texts: &Texts,
        json_lists: bool,
    ) -> Result<Value, String> {
        match texts {
            Texts::Text(name) => Ok(alternatives
                .iter()
                .filter_map(|alternative| alternative.get("const").and_then(Value::as_str))
                .find(|constant| constant.eq_ignore_ascii_case(name))
                .map_or_else(
                    || Value::String(name.clone()),
                    |c| Value::String(c.to_owned()),
                )),
            Texts::Map(entries) if entries.len() == 1 => {
                let (key, child) = &entries[0];
                for alternative in alternatives {
                    let declared = alternative
                        .get("properties")
                        .and_then(Value::as_object)
                        .and_then(|properties| {
                            properties
                                .iter()
                                .find(|(name, _)| name.eq_ignore_ascii_case(key))
                        });
                    if let Some((name, schema)) = declared {
                        let mut object = Map::new();
                        object.insert(name.clone(), self.coerce(schema, child, json_lists)?);
                        return Ok(Value::Object(object));
                    }
                }
                Ok(raw(texts))
            }
            _ => Ok(raw(texts)),
        }
    }

    /// An element of a JSON array in the environment, whose names are matched without case.
    fn coerce_value(&self, node: &Value, item: &Value) -> Value {
        let target = self.resolve(node);
        match (target.get("oneOf").and_then(Value::as_array), item) {
            (Some(alternatives), Value::String(name)) => alternatives
                .iter()
                .filter_map(|alternative| alternative.get("const").and_then(Value::as_str))
                .find(|constant| constant.eq_ignore_ascii_case(name))
                .map_or_else(|| item.clone(), |c| Value::String(c.to_owned())),
            _ => item.clone(),
        }
    }
}

/// Text as it stands, in the shape of the tree it came in.
fn raw(texts: &Texts) -> Value {
    match texts {
        Texts::Text(text) => Value::String(text.clone()),
        Texts::Map(entries) => Value::Object(
            entries
                .iter()
                .map(|(key, child)| (key.clone(), raw(child)))
                .collect(),
        ),
        Texts::Both => Value::Null,
    }
}

/// The names under a prefix, each by the rest of its name.
fn under(values: Vec<(String, String)>, prefix: &str) -> Vec<(String, String)> {
    let prefix = prefix.replace(':', "__");
    values
        .into_iter()
        .filter_map(|(name, value)| {
            if prefix.is_empty() {
                return Some((name, value));
            }
            let head = name.get(..prefix.len())?;
            let rest = name.get(prefix.len()..)?.strip_prefix("__")?;
            head.eq_ignore_ascii_case(&prefix)
                .then(|| (rest.to_owned(), value))
        })
        .collect()
}

/// A fixture's sources as the documents the schema judges, or why one is refused before it is a
/// document.
fn documents(fixture: &Fixture, staged: &Staged, schema: &Schema) -> Result<Vec<Value>, String> {
    let mut documents = Vec::new();
    for source in &fixture.sources {
        let document = match source {
            Source::File(name) | Source::OptionalFile(name) => {
                let path = staged.path(name);
                let text = match std::fs::read_to_string(&path) {
                    Ok(text) => text,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::NotFound
                            && matches!(source, Source::OptionalFile(_)) =>
                    {
                        continue
                    }
                    Err(error) => return Err(error.to_string()),
                };
                section(parse_file(name, &text)?, &fixture.prefix)?
            }
            Source::Document(text) => section(parse_json(text)?, &fixture.prefix)?,
            Source::Environment => {
                if fixture.prefix.is_empty() {
                    return Err("the environment without a prefix".to_owned());
                }
                let variables = under(fixture.environment.clone(), &fixture.prefix);
                Some(schema.coerce(&schema.0, &texts_of(variables), true)?)
            }
            Source::Pairs(pairs) => {
                let pairs = under(pairs.clone(), &fixture.prefix);
                Some(schema.coerce(&schema.0, &texts_of(pairs), false)?)
            }
            Source::PairsJson(text) => {
                let Value::Object(object) = parse_json(text)? else {
                    return Err("pairs that are not an object".to_owned());
                };
                let mut pairs = Vec::new();
                for (name, value) in object {
                    let Value::String(value) = value else {
                        return Err("a pair that is not text".to_owned());
                    };
                    pairs.push((name, value));
                }
                let pairs = under(pairs, &fixture.prefix);
                Some(schema.coerce(&schema.0, &texts_of(pairs), false)?)
            }
        };
        documents.extend(document);
    }
    Ok(documents)
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

/// The schema, with the sizes its `format` words name as bounds, since a validator takes a
/// format for an annotation: an `int32` is an integer of 32 bits, as OpenAPI defines the word.
fn with_formats_as_bounds(mut node: Value) -> Value {
    match &mut node {
        Value::Object(entries) => {
            if let Some(Value::String(format)) = entries.get("format").cloned() {
                let (least, most) = match format.as_str() {
                    "int32" => (json!(i32::MIN), json!(i32::MAX)),
                    "int64" => (json!(i64::MIN), json!(i64::MAX)),
                    _ => (Value::Null, Value::Null),
                };
                if !least.is_null() {
                    let stated = |key: &str| entries.get(key).and_then(Value::as_f64);
                    let number = |bound: &Value| bound.as_f64().expect("a bound is a number");
                    let minimum = match stated("minimum") {
                        Some(stated) if stated >= number(&least) => None,
                        _ => Some(least),
                    };
                    let maximum = match stated("maximum") {
                        Some(stated) if stated <= number(&most) => None,
                        _ => Some(most),
                    };
                    if let Some(bound) = minimum {
                        entries.insert("minimum".to_owned(), bound);
                    }
                    if let Some(bound) = maximum {
                        entries.insert("maximum".to_owned(), bound);
                    }
                }
            }
            for child in entries.values_mut() {
                *child = with_formats_as_bounds(child.take());
            }
        }
        Value::Array(items) => {
            for child in items.iter_mut() {
                *child = with_formats_as_bounds(child.take());
            }
        }
        _ => {}
    }
    node
}

fn validator(schema: &str) -> (Schema, jsonschema::Validator) {
    let schema: Value = serde_json::from_str(schema).expect("the schema is JSON");
    let schema = with_formats_as_bounds(schema);
    let validator = jsonschema::validator_for(&schema).expect("a valid schema");
    (Schema(schema), validator)
}

/// Every fixture is accepted or refused alike by the loader and by the schema.
#[test]
fn the_loader_and_the_runtime_schema_accept_and_refuse_the_same_fixtures() {
    let (schema, validator) = validator(RUNTIME_SCHEMA);
    let mut disagreements = Vec::new();
    // What the validator itself refuses and accepts, so that a test whose documents never reach
    // it cannot pass.
    let (mut validated, mut refused_by_the_validator) = (0, 0);
    for (index, fixture) in fixtures::fixtures().iter().enumerate() {
        let staged = Staged::new(fixture, "schema", index);
        let loaded = configuration(fixture, &staged).load::<RuntimeOptions>();
        let built = documents(fixture, &staged, &schema);
        let mut paths: Vec<String> = Vec::new();
        let judged = built.and_then(|documents| {
            documents.iter().try_for_each(|document| {
                validated += 1;
                paths = validator
                    .iter_errors(document)
                    .map(|error| error.instance_path().to_string())
                    .collect();
                if paths.is_empty() {
                    Ok(())
                } else {
                    refused_by_the_validator += 1;
                    Err(format!("at {}", paths.join(", ")))
                }
            })
        });
        // A refusal that states its key is the schema's too: the validator names that key, or the
        // object or the alternative that holds it.
        if let (Err(_), Outcome::Refused { key: Some(key), .. }, false) =
            (&loaded, &fixture.outcome, paths.is_empty())
        {
            let key: Vec<String> = key.split('.').map(str::to_ascii_lowercase).collect();
            let at_the_key = paths.iter().any(|path| {
                let path: Vec<String> = path
                    .split('/')
                    .skip(1)
                    .map(str::to_ascii_lowercase)
                    .collect();
                // The root holds no alternative, so a refusal there is a key of the root's own.
                key.starts_with(&path) && (!path.is_empty() || key.len() == 1)
            });
            let key = key.join(".");
            if !at_the_key {
                disagreements.push(format!(
                    "{}: the loader refuses {key}, the schema {}",
                    fixture.name,
                    paths.join(", ")
                ));
            }
        }
        if loaded.is_ok() != judged.is_ok() {
            disagreements.push(format!(
                "{}: the loader {}, the schema {}",
                fixture.name,
                match &loaded {
                    Ok(_) => "accepts".to_owned(),
                    Err(refused) => format!("refuses ({refused})"),
                },
                match &judged {
                    Ok(()) => "accepts".to_owned(),
                    Err(refused) => format!("refuses ({refused})"),
                },
            ));
        }
    }
    assert!(
        disagreements.is_empty(),
        "the loader and the schema disagree:\n{}",
        disagreements.join("\n")
    );
    assert!(
        refused_by_the_validator >= 40 && validated >= refused_by_the_validator + 20,
        "at least 40 documents have to be refused by the validator itself, and at least 20 \
         more judged, or the test shows nothing: it judged {validated} and refused \
         {refused_by_the_validator}"
    );
}

/// A channel's own document is judged by the channel schema as by the loader.
#[test]
fn the_loader_and_the_channel_schema_accept_and_refuse_the_same_documents() {
    let (_, validator) = validator(CHANNEL_SCHEMA);
    let mut disagreements = Vec::new();
    for document in [
        r#"{}"#,
        r#"{"Grpc":{"UserAgent":"armonik"}}"#,
        r#"{"Grpc":{"UserAgent":""}}"#,
        r#"{"UserAgnt":"typo"}"#,
        r#"{"Grpc":{"UserAgnt":"typo"}}"#,
        r#"{"Grpc":{"Host":{"Receive":{"Window":2}}}}"#,
        r#"{"Grpc":{"Host":{"Receive":{"Window":0}}}}"#,
        r#"{"Grpc":{"Host":{"Receive":{"Window":"2"}}}}"#,
        r#"{"Http2":{"Send":{"FramesPerWrite":257}}}"#,
        r#"{"Http2":{"KeepAlive":{"Ping":{"IntervalSeconds":0}}}}"#,
        r#"{"Http2":{"KeepAlive":{"Ping":{}}}}"#,
        r#"{"Transport":{"TcpKeepalive":{"Probe":{"IdleSeconds":32768}}}}"#,
    ] {
        let loaded = Configuration::with_prefix("")
            .document(document)
            .load::<ChannelOptions>();
        let parsed: Value = serde_json::from_str(document).expect("a document");
        if loaded.is_ok() != validator.is_valid(&parsed) {
            disagreements.push(format!("{document}: the loader says {loaded:?}"));
        }
    }
    assert!(
        disagreements.is_empty(),
        "the loader and the schema disagree:\n{}",
        disagreements.join("\n")
    );
}
