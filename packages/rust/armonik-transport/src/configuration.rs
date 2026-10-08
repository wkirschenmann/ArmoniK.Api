//! The configuration loader: the sources a host lists, read into one document.
//!
//! A source is a file in JSON, YAML or TOML, the process environment under a prefix, pairs of a
//! key's path and a text value, or a JSON document. Each is read into the document's type, and a
//! later one is merged over an earlier one by the type's own [`Document::over`], so that what the
//! sources mean does not depend on who listed them.
//!
//! A key the document does not declare is not refused. It is logged, with its source and its path,
//! and the load goes on, so that a configuration written for a later engine still loads and a
//! misspelled key is still said. A value that does not fit its key's type is refused, by its source
//! and its path, and never quoted: a password is a value.

use std::cell::RefCell;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::de::{
    self, DeserializeSeed, Deserializer, EnumAccess, IntoDeserializer, MapAccess, SeqAccess,
    Unexpected, VariantAccess, Visitor,
};

/// The prefix a configuration is read under when the host names none.
///
/// `ArmoniK__Client__Grpc`: the gRPC part of the ArmoniK client's configuration. Its parts are
/// joined by `__`, as an environment variable's name writes them, and a file holds them as nested
/// sections.
pub const DEFAULT_PREFIX: &str = "ArmoniK__Client__Grpc";

/// The separator between the parts of a key's path in the environment and in pairs, as .NET's
/// configuration providers write it.
const SEPARATOR: &str = "__";

/// A document a configuration can be read into, merged one over another.
pub trait Document: serde::de::DeserializeOwned + Default {
    /// This document over `earlier`: what this one states wins, and what it leaves out is the
    /// earlier one's.
    fn over(self, earlier: Self) -> Self;
}

/// Where a configuration comes from, in the order the sources are added.
#[derive(Clone)]
pub struct Configuration {
    prefix: Option<String>,
    sources: Vec<Source>,
}

/// The sources by what they are called, never by what they hold: pairs and documents carry
/// passwords as readily as any other value.
impl fmt::Debug for Configuration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Configuration")
            .field("prefix", &self.prefix)
            .field(
                "sources",
                &self
                    .sources
                    .iter()
                    .map(|source| SourceName::of(source).to_string())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

#[derive(Clone)]
enum Source {
    File { path: PathBuf, optional: bool },
    Environment,
    Pairs(Vec<(String, String)>),
    PairsJson(String),
    Document(String),
}

impl Default for Configuration {
    fn default() -> Self {
        Self::new()
    }
}

impl Configuration {
    /// Under [`DEFAULT_PREFIX`], with no source.
    pub fn new() -> Self {
        Self::with_prefix(DEFAULT_PREFIX)
    }

    /// Under `prefix`, or under none when it is empty: a file's document is then the whole file.
    ///
    /// The prefix is a path, its parts joined by `__`, or by `:` as a .NET section's path is
    /// written, which is read as `__`.
    pub fn with_prefix(prefix: &str) -> Self {
        Self {
            prefix: (!prefix.is_empty()).then(|| prefix.replace(':', SEPARATOR)),
            sources: Vec::new(),
        }
    }

    /// A file, JSON, YAML or TOML by its extension, refused when it does not exist.
    pub fn file(self, path: impl Into<PathBuf>) -> Self {
        self.with(Source::File {
            path: path.into(),
            optional: false,
        })
    }

    /// A file, JSON, YAML or TOML by its extension, which contributes nothing when it does not
    /// exist.
    pub fn optional_file(self, path: impl Into<PathBuf>) -> Self {
        self.with(Source::File {
            path: path.into(),
            optional: true,
        })
    }

    /// The variables of the process environment whose name starts with the prefix and `__`, read
    /// when the configuration is loaded. Refused at the load when there is no prefix.
    pub fn environment(self) -> Self {
        self.with(Source::Environment)
    }

    /// Pairs of a key's path, its parts joined by `__` under no prefix, and a text value, read as
    /// the environment's are.
    pub fn pairs(self, pairs: impl IntoIterator<Item = (String, String)>) -> Self {
        self.with(Source::Pairs(pairs.into_iter().collect()))
    }

    /// Pairs as a JSON object whose names are keys' paths and whose values are text. Read as
    /// [`Configuration::pairs`] reads its own, and refused at the load when it is not such an
    /// object.
    pub fn pairs_json(self, json: impl Into<String>) -> Self {
        self.with(Source::PairsJson(json.into()))
    }

    /// A JSON document in the document's vocabulary, with no prefix around it.
    pub fn document(self, json: impl Into<String>) -> Self {
        self.with(Source::Document(json.into()))
    }

    fn with(mut self, source: Source) -> Self {
        self.sources.push(source);
        self
    }

    /// Reads the sources, in order, into one document, each key it does not declare logged.
    ///
    /// The first refusal ends the load. With no source, or none that contributes, the document is
    /// its type's default.
    pub fn load<D: Document>(&self) -> Result<D, ConfigRefusal> {
        let mut loaded: Option<D> = None;
        for source in &self.sources {
            let named = SourceName::of(source);
            let Some(tree) = self.tree(source, &named)? else {
                continue;
            };
            let document: D = read(tree, &named)?;
            loaded = Some(match loaded {
                Some(earlier) => document.over(earlier),
                None => document,
            });
        }
        Ok(loaded.unwrap_or_default())
    }

    /// What a source states, as a tree, or nothing when it contributes nothing.
    fn tree(&self, source: &Source, named: &SourceName) -> Result<Option<Tree>, ConfigRefusal> {
        let refused = |why: String| ConfigRefusal::new(named.clone(), None, why);
        match source {
            Source::File { path, optional } => {
                let text = match std::fs::read_to_string(path) {
                    Ok(text) => text,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound && *optional => {
                        return Ok(None)
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        return Err(refused("the file does not exist".to_owned()))
                    }
                    Err(error) => {
                        return Err(refused(format!("the file could not be read: {error}")))
                    }
                };
                // A byte order mark, as an editor on Windows writes one.
                let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
                let root = parse_file(path, text).map_err(refused)?;
                if !matches!(root, Node::Map(_)) {
                    return Err(refused("the file holds no object".to_owned()));
                }
                let section = match &self.prefix {
                    None => Some(root),
                    Some(prefix) => section(root, prefix)
                        .map_err(|at| refused(format!("its section {at} is not an object")))?,
                };
                Ok(section.map(Tree::typed))
            }
            Source::Environment => {
                let Some(prefix) = &self.prefix else {
                    return Err(refused(
                        "the environment needs a prefix: with none, every variable of the \
                         process would be a key"
                            .to_owned(),
                    ));
                };
                let mut variables = Vec::new();
                for (name, value) in std::env::vars_os() {
                    let Some(name) = name.to_str() else {
                        continue;
                    };
                    let Some(path) = under(name, prefix) else {
                        continue;
                    };
                    let Ok(value) = value.into_string() else {
                        return Err(ConfigRefusal::new(
                            named.clone(),
                            Some(dotted(path)),
                            "its value is not Unicode".to_owned(),
                        ));
                    };
                    variables.push((path.to_owned(), value));
                }
                // By name, so that two names one key spells in two cases are taken in an order
                // that does not change from one run to the next.
                variables.sort();
                Ok(Some(Texts::from(variables, named)))
            }
            Source::Pairs(pairs) => Ok(Some(Texts::from(pairs.clone(), named))),
            Source::PairsJson(json) => {
                let not_pairs = || refused("they are not a JSON object of text values".to_owned());
                let parsed: Node = serde_json::from_str(json)
                    .map_err(|error| refused(format!("they are not JSON: {error}")))?;
                let Node::Map(pairs) = parsed else {
                    return Err(not_pairs());
                };
                let pairs = pairs
                    .into_iter()
                    .map(|(path, value)| match value {
                        Node::Text(value) => Ok((path, value)),
                        _ => Err(not_pairs()),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Some(Texts::from(pairs, named)))
            }
            Source::Document(json) => {
                let parsed: Node = serde_json::from_str(json)
                    .map_err(|error| refused(format!("it is not JSON: {error}")))?;
                match parsed {
                    root @ Node::Map(_) => Ok(Some(Tree::typed(root))),
                    _ => Err(refused("it is not a JSON object".to_owned())),
                }
            }
        }
    }
}

/// The section a prefix names in a file's tree: its parts, joined by `__`, walk down the nested
/// sections, each key compared as written, so that `A__B` is the section `B` of the section `A`.
/// Nothing when a section is missing; an error, the path to the section at fault, when a section
/// is not an object.
fn section(root: Node, prefix: &str) -> Result<Option<Node>, String> {
    let mut current = root;
    let mut walked = 0;
    for part in prefix.split(SEPARATOR) {
        let Node::Map(entries) = current else {
            return Err(prefix[..walked].trim_end_matches(SEPARATOR).to_owned());
        };
        match entries.into_iter().find(|(key, _)| key == part) {
            Some((_, found)) => current = found,
            None => return Ok(None),
        }
        walked += part.len() + SEPARATOR.len();
    }
    match current {
        Node::Map(_) => Ok(Some(current)),
        _ => Err(prefix.to_owned()),
    }
}

/// The rest of a variable's name past `prefix` and the separator, compared without case, as the
/// environment of Windows and .NET's providers compare names.
fn under<'a>(name: &'a str, prefix: &str) -> Option<&'a str> {
    let head = name.get(..prefix.len())?;
    let rest = name.get(prefix.len()..)?;
    if !head.eq_ignore_ascii_case(prefix) {
        return None;
    }
    rest.strip_prefix(SEPARATOR)
}

/// A path written with the separator, as a message names it.
fn dotted(path: &str) -> String {
    path.split(SEPARATOR).collect::<Vec<_>>().join(".")
}

/// A file's content as a tree, by its extension; a refusal names the line.
fn parse_file(path: &Path, text: &str) -> Result<Node, String> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("json") => {
            serde_json::from_str::<Node>(text).map_err(|error| format!("it is not JSON: {error}"))
        }
        Some("yaml" | "yml") => {
            let documents = yaml_rust2::YamlLoader::load_from_str(text).map_err(|error| {
                format!(
                    "it is not YAML: {} at line {} column {}",
                    error.info(),
                    error.marker().line(),
                    error.marker().col() + 1
                )
            })?;
            // One document: a second one would be a configuration read in part, with nothing to
            // say which part.
            if documents.len() > 1 {
                return Err("it holds more than one YAML document".to_owned());
            }
            documents
                .into_iter()
                .next()
                .map_or(Ok(Node::Map(Vec::new())), Node::from_yaml)
        }
        Some("toml") => toml::from_str::<toml::Table>(text)
            .map(|table| Node::from_toml(toml::Value::Table(table)))
            .map_err(|error| {
                let line = error
                    .span()
                    .and_then(|span| text.get(..span.start))
                    .map(|before| before.matches('\n').count() + 1);
                match line {
                    Some(line) => format!("it is not TOML: {} at line {line}", error.message()),
                    None => format!("it is not TOML: {}", error.message()),
                }
            }),
        _ => Err("its extension is none of .json, .yaml, .yml and .toml".to_owned()),
    }
}

/// What a source is called in a refusal and in the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceName {
    File(PathBuf),
    Environment,
    Pairs,
    Document,
}

impl SourceName {
    fn of(source: &Source) -> Self {
        match source {
            Source::File { path, .. } => Self::File(path.clone()),
            Source::Environment => Self::Environment,
            Source::Pairs(_) | Source::PairsJson(_) => Self::Pairs,
            Source::Document(_) => Self::Document,
        }
    }
}

impl fmt::Display for SourceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File(path) => write!(f, "{}", path.display()),
            Self::Environment => f.write_str("the environment"),
            Self::Pairs => f.write_str("pairs"),
            Self::Document => f.write_str("a document"),
        }
    }
}

/// Why a configuration was not loaded: the source that refused, the key at fault when one is,
/// and why. A value is never quoted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigRefusal {
    source: SourceName,
    key: Option<String>,
    why: String,
}

impl ConfigRefusal {
    fn new(source: SourceName, key: Option<String>, why: String) -> Self {
        Self { source, key, why }
    }

    /// The source that refused.
    pub fn source_name(&self) -> &SourceName {
        &self.source
    }

    /// The path of the key at fault within the prefix, its parts joined by `.`, when a key is.
    pub fn key(&self) -> Option<&str> {
        self.key.as_deref()
    }
}

impl fmt::Display for ConfigRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.key, &self.source) {
            (Some(key), source) => write!(f, "{source}: {key} is refused: {}", self.why),
            (None, SourceName::Pairs) => write!(f, "pairs are refused: {}", self.why),
            (None, source) => write!(f, "{source} is refused: {}", self.why),
        }
    }
}

impl std::error::Error for ConfigRefusal {}

/// A source's tree, and whether its values are typed or text to be parsed by their key's type.
struct Tree {
    root: Node,
    text: bool,
}

impl Tree {
    fn typed(root: Node) -> Self {
        Self { root, text: false }
    }
}

/// What a source states, before it is read into a document.
#[derive(Debug, Clone)]
enum Node {
    Null,
    Bool(bool),
    Unsigned(u64),
    Signed(i64),
    Float(f64),
    Text(String),
    List(Vec<Node>),
    Map(Vec<(String, Node)>),
    /// A key the environment or pairs give both a value and keys under it.
    Both,
}

impl Node {
    /// A YAML document as a tree. The parser refuses a key stated twice; what it could not read,
    /// a value or a key that is not a scalar, is refused here rather than taken for nothing.
    fn from_yaml(value: yaml_rust2::Yaml) -> Result<Self, String> {
        use yaml_rust2::Yaml;
        Ok(match value {
            Yaml::Null => Self::Null,
            Yaml::BadValue | Yaml::Alias(_) => {
                return Err("it holds a value YAML could not read".to_owned())
            }
            Yaml::Boolean(value) => Self::Bool(value),
            Yaml::Integer(value) => match u64::try_from(value) {
                Ok(value) => Self::Unsigned(value),
                Err(_) => Self::Signed(value),
            },
            // The parser keeps a real as the text it read, which it has checked is one.
            Yaml::Real(text) => text.parse().map_or(Self::Text(text), Self::Float),
            Yaml::String(value) => Self::Text(value),
            Yaml::Array(items) => Self::List(
                items
                    .into_iter()
                    .map(Self::from_yaml)
                    .collect::<Result<_, _>>()?,
            ),
            Yaml::Hash(entries) => Self::Map(
                entries
                    .into_iter()
                    .map(|(key, value)| Ok((yaml_key(key)?, Self::from_yaml(value)?)))
                    .collect::<Result<_, String>>()?,
            ),
        })
    }

    fn from_toml(value: toml::Value) -> Self {
        use toml::Value;
        match value {
            Value::Boolean(value) => Self::Bool(value),
            Value::Integer(value) => match u64::try_from(value) {
                Ok(value) => Self::Unsigned(value),
                Err(_) => Self::Signed(value),
            },
            Value::Float(value) => Self::Float(value),
            Value::String(value) => Self::Text(value),
            Value::Datetime(value) => Self::Text(value.to_string()),
            Value::Array(items) => Self::List(items.into_iter().map(Self::from_toml).collect()),
            Value::Table(entries) => Self::Map(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, Self::from_toml(value)))
                    .collect(),
            ),
        }
    }

    /// What it is, as a refusal names it without quoting it.
    fn unexpected(&self) -> Unexpected<'static> {
        match self {
            Self::Null => Unexpected::Unit,
            Self::Bool(_) => Unexpected::Bool(false),
            Self::Unsigned(_) => Unexpected::Unsigned(0),
            Self::Signed(_) => Unexpected::Signed(0),
            Self::Float(_) => Unexpected::Float(0.0),
            Self::Text(_) => Unexpected::Str(""),
            Self::List(_) => Unexpected::Seq,
            Self::Map(_) | Self::Both => Unexpected::Map,
        }
    }
}

/// A JSON text read as written, a key stated twice refused: a map would keep one of the two values
/// and drop the other in silence.
impl<'de> serde::Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Any;

        impl<'de> Visitor<'de> for Any {
            type Value = Node;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a JSON value")
            }

            fn visit_unit<E: de::Error>(self) -> Result<Node, E> {
                Ok(Node::Null)
            }

            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Node, E> {
                Ok(Node::Bool(value))
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Node, E> {
                Ok(Node::Unsigned(value))
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Node, E> {
                Ok(Node::Signed(value))
            }

            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Node, E> {
                Ok(Node::Float(value))
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Node, E> {
                Ok(Node::Text(value.to_owned()))
            }

            fn visit_string<E: de::Error>(self, value: String) -> Result<Node, E> {
                Ok(Node::Text(value))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut items: A) -> Result<Node, A::Error> {
                let mut read = Vec::new();
                while let Some(item) = items.next_element()? {
                    read.push(item);
                }
                Ok(Node::List(read))
            }

            fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> Result<Node, A::Error> {
                let mut read: Vec<(String, Node)> = Vec::new();
                while let Some(key) = entries.next_key::<String>()? {
                    if read.iter().any(|(known, _)| *known == key) {
                        return Err(de::Error::custom(format_args!("{key} is stated twice")));
                    }
                    let value = entries.next_value()?;
                    read.push((key, value));
                }
                Ok(Node::Map(read))
            }
        }

        deserializer.deserialize_any(Any)
    }
}

/// A YAML key as the text a document names a key by.
fn yaml_key(key: yaml_rust2::Yaml) -> Result<String, String> {
    use yaml_rust2::Yaml;
    match key {
        Yaml::String(text) | Yaml::Real(text) => Ok(text),
        Yaml::Integer(value) => Ok(value.to_string()),
        Yaml::Boolean(value) => Ok(value.to_string()),
        _ => Err("it holds a key that is not a scalar".to_owned()),
    }
}

/// The text values of the environment or of pairs, gathered into a tree by their paths.
///
/// The parts of a path are compared without case, so two spellings of one key are one key; the
/// spelling kept is the first one met, and the reader matches it to the document's own.
#[derive(Default)]
struct Texts {
    root: Vec<(String, Node)>,
}

impl Texts {
    /// The tree of `values`, a key given twice taken from the later and logged.
    fn from(values: Vec<(String, String)>, source: &SourceName) -> Tree {
        let mut texts = Self::default();
        for (path, value) in values {
            let parts: Vec<&str> = path.split(SEPARATOR).collect();
            if Self::place(&mut texts.root, &parts, value) {
                tracing::warn!(
                    source = %source,
                    key = %dotted(&path),
                    "the configuration gives a key twice, which is taken from the later"
                );
            }
        }
        Tree {
            root: Node::Map(texts.root),
            text: true,
        }
    }

    /// Whether `parts` already held a value, which this one takes the place of.
    fn place(entries: &mut Vec<(String, Node)>, parts: &[&str], value: String) -> bool {
        let Some((first, rest)) = parts.split_first() else {
            return false;
        };
        let at = match entries
            .iter()
            .position(|(key, _)| key.eq_ignore_ascii_case(first))
        {
            Some(at) => at,
            None => {
                entries.push(((*first).to_owned(), Node::Map(Vec::new())));
                entries.len() - 1
            }
        };
        let node = &mut entries[at].1;
        if rest.is_empty() {
            let twice = matches!(node, Node::Text(_));
            *node = match node {
                Node::Map(children) if children.is_empty() => Node::Text(value),
                Node::Map(_) | Node::Both => Node::Both,
                _ => Node::Text(value),
            };
            return twice;
        }
        match node {
            Node::Map(children) => Self::place(children, rest, value),
            _ => {
                *node = Node::Both;
                false
            }
        }
    }
}

/// Reads one source's tree into the document, each key it does not declare logged.
fn read<D: serde::de::DeserializeOwned>(
    tree: Tree,
    source: &SourceName,
) -> Result<D, ConfigRefusal> {
    let ignored = RefCell::new(Vec::new());
    let read = D::deserialize(Reader {
        node: tree.root,
        path: String::new(),
        text: tree.text,
        ignored: &ignored,
    });
    for key in ignored.into_inner() {
        tracing::info!(
            source = %source,
            key = %key,
            "the configuration names a key the engine does not know, which is ignored"
        );
    }
    read.map_err(|refused| ConfigRefusal::new(source.clone(), refused.key, refused.why))
}

/// Why a tree was not read, and where in it.
#[derive(Debug)]
struct Refused {
    key: Option<String>,
    why: String,
}

impl Refused {
    /// The same refusal, at `path` unless a deeper key already named it.
    fn at(mut self, path: &str) -> Self {
        if self.key.is_none() && !path.is_empty() {
            self.key = Some(path.to_owned());
        }
        self
    }
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.why)
    }
}

impl std::error::Error for Refused {}

/// Every message serde builds goes through here, and none quotes the value it was given.
impl de::Error for Refused {
    fn custom<T: fmt::Display>(why: T) -> Self {
        Self {
            key: None,
            why: why.to_string(),
        }
    }

    fn invalid_type(found: Unexpected, expected: &dyn de::Expected) -> Self {
        Self::custom(format_args!(
            "it is {}, where {expected} is expected",
            kind(found)
        ))
    }

    fn invalid_value(_: Unexpected, expected: &dyn de::Expected) -> Self {
        Self::custom(format_args!("its value is not {expected}"))
    }

    fn invalid_length(length: usize, expected: &dyn de::Expected) -> Self {
        Self::custom(format_args!(
            "it holds {length}, where {expected} is expected"
        ))
    }

    fn unknown_variant(_: &str, expected: &'static [&'static str]) -> Self {
        Self::custom(format_args!("it names none of {}", expected.join(", ")))
    }

    fn missing_field(field: &'static str) -> Self {
        Self::custom(format_args!("it lacks {field}"))
    }

    fn duplicate_field(field: &'static str) -> Self {
        Self::custom(format_args!("it states {field} twice"))
    }
}

/// The kind of a value, without the value.
fn kind(found: Unexpected) -> &'static str {
    match found {
        Unexpected::Bool(_) => "a boolean",
        Unexpected::Unsigned(_) | Unexpected::Signed(_) | Unexpected::Float(_) => "a number",
        Unexpected::Char(_) | Unexpected::Str(_) => "a text",
        Unexpected::Bytes(_) => "bytes",
        Unexpected::Unit => "null",
        Unexpected::Seq => "a list",
        Unexpected::Map => "an object",
        _ => "another kind of value",
    }
}

fn both() -> Refused {
    de::Error::custom("it is given a value and keys under it at once")
}

fn joined(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

/// A node read by the type a document gives its key: typed, as a file or a document holds it, or
/// text, as the environment and pairs hold it, parsed by that type.
struct Reader<'a> {
    node: Node,
    path: String,
    text: bool,
    ignored: &'a RefCell<Vec<String>>,
}

impl<'a> Reader<'a> {
    /// The text of a text source's value, which `parse` turns into what its key holds.
    fn parsed<T: std::str::FromStr>(&self, what: &str) -> Option<Result<T, Refused>> {
        match (&self.node, self.text) {
            (Node::Text(text), true) => {
                Some(text.trim().parse().map_err(|_| {
                    de::Error::custom(format_args!("it is a text that is not {what}"))
                }))
            }
            _ => None,
        }
    }
}

macro_rules! text_or_any {
    ($($method:ident => $parsed:ty, $visit:ident, $what:literal;)+) => {
        $(
            fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Refused> {
                match self.parsed::<$parsed>($what) {
                    Some(parsed) => visitor.$visit(parsed?),
                    None => self.deserialize_any(visitor),
                }
            }
        )+
    };
}

impl<'de, 'a> Deserializer<'de> for Reader<'a> {
    type Error = Refused;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Refused> {
        let text = self.text;
        let ignored = self.ignored;
        let path = self.path;
        match self.node {
            Node::Null => visitor.visit_unit(),
            Node::Bool(value) => visitor.visit_bool(value),
            Node::Unsigned(value) => visitor.visit_u64(value),
            Node::Signed(value) => visitor.visit_i64(value),
            Node::Float(value) => visitor.visit_f64(value),
            Node::Text(value) => visitor.visit_string(value),
            Node::List(items) => visitor.visit_seq(Items {
                items: items.into_iter().enumerate(),
                path,
                text,
                ignored,
            }),
            Node::Map(entries) => visitor.visit_map(Entries {
                entries: entries.into_iter(),
                fields: None,
                pending: None,
                path,
                text,
                ignored,
            }),
            Node::Both => Err(both()),
        }
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Refused> {
        match (&self.node, self.text) {
            (Node::Text(text), true) => {
                let text = text.trim();
                if text.eq_ignore_ascii_case("true") {
                    visitor.visit_bool(true)
                } else if text.eq_ignore_ascii_case("false") {
                    visitor.visit_bool(false)
                } else {
                    Err(de::Error::custom("it is a text that is not true or false"))
                }
            }
            _ => self.deserialize_any(visitor),
        }
    }

    text_or_any! {
        deserialize_i8 => i64, visit_i64, "an integer";
        deserialize_i16 => i64, visit_i64, "an integer";
        deserialize_i32 => i64, visit_i64, "an integer";
        deserialize_i64 => i64, visit_i64, "an integer";
        deserialize_u8 => u64, visit_u64, "a positive integer";
        deserialize_u16 => u64, visit_u64, "a positive integer";
        deserialize_u32 => u64, visit_u64, "a positive integer";
        deserialize_u64 => u64, visit_u64, "a positive integer";
        deserialize_f32 => f64, visit_f64, "a number";
        deserialize_f64 => f64, visit_f64, "a number";
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Refused> {
        match self.node {
            Node::Null => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        visitor: V,
    ) -> Result<V::Value, Refused> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Refused> {
        match self.node {
            Node::Map(entries) => visitor.visit_map(Entries {
                entries: entries.into_iter(),
                fields: Some(fields),
                pending: None,
                path: self.path,
                text: self.text,
                ignored: self.ignored,
            }),
            Node::Both => Err(both()),
            node => Err(de::Error::invalid_type(node.unexpected(), &visitor)),
        }
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Refused> {
        match self.node {
            Node::Text(name) => visitor.visit_enum(Variant {
                name: spelled(name, variants, self.text),
                value: None,
                path: self.path,
                text: self.text,
                ignored: self.ignored,
            }),
            Node::Map(entries) if entries.len() == 1 => {
                let (name, value) = entries.into_iter().next().expect("one entry");
                visitor.visit_enum(Variant {
                    name: spelled(name, variants, self.text),
                    value: Some(value),
                    path: self.path,
                    text: self.text,
                    ignored: self.ignored,
                })
            }
            Node::Both => Err(both()),
            node => Err(de::Error::invalid_type(node.unexpected(), &visitor)),
        }
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Refused> {
        self.ignored.borrow_mut().push(self.path);
        visitor.visit_unit()
    }

    serde::forward_to_deserialize_any! {
        i128 u128 char str string bytes byte_buf unit unit_struct seq tuple tuple_struct map
        identifier
    }
}

/// A name as the document spells it, where a text source spelled it in another case.
fn spelled(name: String, known: &'static [&'static str], text: bool) -> String {
    if !text || known.contains(&name.as_str()) {
        return name;
    }
    known
        .iter()
        .find(|known| known.eq_ignore_ascii_case(&name))
        .map_or(name, |known| (*known).to_owned())
}

struct Entries<'a> {
    entries: std::vec::IntoIter<(String, Node)>,
    fields: Option<&'static [&'static str]>,
    pending: Option<(String, Node)>,
    path: String,
    text: bool,
    ignored: &'a RefCell<Vec<String>>,
}

impl<'de, 'a> MapAccess<'de> for Entries<'a> {
    type Error = Refused;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Refused> {
        let Some((key, node)) = self.entries.next() else {
            return Ok(None);
        };
        let key = match self.fields {
            Some(fields) => spelled(key, fields, self.text),
            None => key,
        };
        let read = seed.deserialize(key.clone().into_deserializer());
        self.pending = Some((key, node));
        read.map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Refused> {
        let (key, node) = self
            .pending
            .take()
            .ok_or_else(|| de::Error::custom("a value asked for before its key"))?;
        let path = joined(&self.path, &key);
        seed.deserialize(Reader {
            node,
            path: path.clone(),
            text: self.text,
            ignored: self.ignored,
        })
        .map_err(|refused| refused.at(&path))
    }
}

struct Items<'a> {
    items: std::iter::Enumerate<std::vec::IntoIter<Node>>,
    path: String,
    text: bool,
    ignored: &'a RefCell<Vec<String>>,
}

impl<'de, 'a> SeqAccess<'de> for Items<'a> {
    type Error = Refused;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Refused> {
        let Some((index, node)) = self.items.next() else {
            return Ok(None);
        };
        let path = joined(&self.path, &index.to_string());
        seed.deserialize(Reader {
            node,
            path: path.clone(),
            text: self.text,
            ignored: self.ignored,
        })
        .map(Some)
        .map_err(|refused| refused.at(&path))
    }
}

/// One variant of an enum, named by a text or by the one key of an object.
struct Variant<'a> {
    name: String,
    value: Option<Node>,
    path: String,
    text: bool,
    ignored: &'a RefCell<Vec<String>>,
}

impl<'a> Variant<'a> {
    fn content(self) -> Option<Reader<'a>> {
        let path = joined(&self.path, &self.name);
        self.value.map(|node| Reader {
            node,
            path,
            text: self.text,
            ignored: self.ignored,
        })
    }
}

impl<'de, 'a> EnumAccess<'de> for Variant<'a> {
    type Error = Refused;
    type Variant = Self;

    fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, Self), Refused> {
        let name = seed.deserialize(self.name.clone().into_deserializer())?;
        Ok((name, self))
    }
}

impl<'de, 'a> VariantAccess<'de> for Variant<'a> {
    type Error = Refused;

    fn unit_variant(self) -> Result<(), Refused> {
        match self.value {
            None | Some(Node::Null) => Ok(()),
            Some(_) => Err(de::Error::custom(
                "it carries a value, where none is expected",
            )),
        }
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, Refused> {
        let path = joined(&self.path, &self.name);
        match self.content() {
            Some(reader) => seed
                .deserialize(reader)
                .map_err(|refused| refused.at(&path)),
            None => Err(de::Error::invalid_type(Unexpected::UnitVariant, &"a value")),
        }
    }

    fn tuple_variant<V: Visitor<'de>>(self, _: usize, visitor: V) -> Result<V::Value, Refused> {
        match self.content() {
            Some(reader) => reader.deserialize_seq(visitor),
            None => Err(de::Error::invalid_type(Unexpected::UnitVariant, &visitor)),
        }
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Refused> {
        match self.content() {
            Some(reader) => reader.deserialize_struct("", fields, visitor),
            None => Err(de::Error::invalid_type(Unexpected::UnitVariant, &visitor)),
        }
    }
}
