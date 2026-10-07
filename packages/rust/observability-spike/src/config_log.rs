//! Point 7: the effective configuration, logged through each type's own `Debug`, and the check
//! that finds a secret a type renders in plain text.

use armonik_transport::options::ChannelOptions;
use serde_json::{Map, Value};

/// What the runtime holds once its options are read: the shape `ak_runtime_create` logs.
#[derive(Debug, Default)]
pub struct Effective {
    pub memory_ceiling: Option<u64>,
    pub memory_hard_ceiling: Option<u64>,
    pub channel_defaults: Option<ChannelOptions>,
}

/// Logged at info, once, when the runtime is created.
///
/// One field per top-level option, each rendered by `Debug`: a type that holds a secret renders it
/// redacted (`Password`, `CredentialedUrl`, `ProxyUrl`), and a field added to an options type is
/// in the output without a line here.
pub fn log_effective(effective: &Effective) {
    tracing::info!(
        target: "armonik_transport_ffi::config",
        memory_ceiling = ?effective.memory_ceiling,
        memory_hard_ceiling = ?effective.memory_hard_ceiling,
        channel_defaults = ?effective.channel_defaults,
        "effective configuration"
    );
}

/// An unknown key: its source and its path, never its value.
pub fn log_unknown_key(source: &str, path: &str) {
    tracing::warn!(
        target: "armonik_transport_ffi::config",
        source,
        path,
        "unknown configuration key ignored"
    );
}

/// One string leaf of a schema: where it is, and whether the schema declares it a secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Leaf {
    /// Property names from the root, `.`-joined; an alternative of a `oneOf` is its one property.
    pub path: String,
    pub write_only: bool,
}

/// The string leaves of `schema`, followed through `$ref` and every `oneOf` alternative.
pub fn string_leaves(schema: &Value) -> Vec<Leaf> {
    let mut leaves = Vec::new();
    walk(schema, schema, "", false, &mut leaves, 0);
    leaves.sort_by(|a, b| a.path.cmp(&b.path));
    leaves.dedup();
    leaves
}

fn resolve<'a>(root: &'a Value, node: &'a Value) -> &'a Value {
    match node.get("$ref").and_then(Value::as_str) {
        Some(reference) => {
            let name = reference.rsplit('/').next().unwrap_or_default();
            &root["$defs"][name]
        }
        None => node,
    }
}

fn walk(root: &Value, node: &Value, path: &str, write_only: bool, out: &mut Vec<Leaf>, depth: usize) {
    if depth > 12 {
        return;
    }
    let write_only = write_only || node.get("writeOnly").and_then(Value::as_bool) == Some(true);
    let node = resolve(root, node);
    let write_only = write_only || node.get("writeOnly").and_then(Value::as_bool) == Some(true);
    if let Some(alternatives) = node.get("oneOf").and_then(Value::as_array) {
        for alternative in alternatives {
            walk(root, alternative, path, write_only, out, depth + 1);
        }
        return;
    }
    if let Some(properties) = node.get("properties").and_then(Value::as_object) {
        for (name, child) in properties {
            let child_path = if path.is_empty() {
                name.clone()
            } else {
                format!("{path}.{name}")
            };
            walk(root, child, &child_path, write_only, out, depth + 1);
        }
        return;
    }
    let is_string = match node.get("type") {
        Some(Value::String(kind)) => kind == "string",
        Some(Value::Array(kinds)) => kinds.iter().any(|kind| kind == "string"),
        _ => false,
    };
    if is_string && node.get("enum").is_none() && node.get("const").is_none() {
        out.push(Leaf {
            path: path.to_owned(),
            write_only,
        });
    }
}

/// The smallest document that sets this leaf: what the path names, and what each object on the
/// way requires beside it.
pub fn document_setting(root: &Value, leaf: &Leaf, value: &str) -> Value {
    let segments: Vec<&str> = leaf.path.split('.').collect();
    build(root, root, &segments, value, 0)
}

fn build(root: &Value, node: &Value, path: &[&str], value: &str, depth: usize) -> Value {
    let node = resolve(root, node);
    if path.is_empty() {
        return Value::String(value.to_owned());
    }
    if let Some(alternatives) = node.get("oneOf").and_then(Value::as_array) {
        let chosen = alternatives
            .iter()
            .find(|alternative| alternative["properties"].get(path[0]).is_some())
            .unwrap_or(&alternatives[0]);
        return build(root, chosen, path, value, depth + 1);
    }
    let mut object = Map::new();
    let properties = node.get("properties").and_then(Value::as_object);
    if let Some(child) = properties.and_then(|properties| properties.get(path[0])) {
        object.insert(
            path[0].to_owned(),
            build(root, child, &path[1..], value, depth + 1),
        );
    }
    for required in node
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|name| *name != path[0])
    {
        if let Some(child) = properties.and_then(|properties| properties.get(required)) {
            object.insert(required.to_owned(), placeholder(root, child, depth + 1));
        }
    }
    Value::Object(object)
}

/// Any value the schema admits, for a property that has to be there.
fn placeholder(root: &Value, node: &Value, depth: usize) -> Value {
    let node = resolve(root, node);
    if depth > 12 {
        return Value::Null;
    }
    if let Some(alternatives) = node.get("oneOf").and_then(Value::as_array) {
        return placeholder(root, &alternatives[0], depth + 1);
    }
    if let Some(value) = node.get("const") {
        return value.clone();
    }
    if let Some(properties) = node.get("properties").and_then(Value::as_object) {
        let required: Vec<&str> = node
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let mut object = Map::new();
        for name in required {
            if let Some(child) = properties.get(name) {
                object.insert(name.to_owned(), placeholder(root, child, depth + 1));
            }
        }
        return Value::Object(object);
    }
    match node.get("type").and_then(Value::as_str) {
        Some("string") => Value::String("placeholder".to_owned()),
        Some("boolean") => Value::Bool(false),
        Some("integer") | Some("number") => node
            .get("minimum")
            .cloned()
            .unwrap_or_else(|| Value::from(1)),
        _ => Value::Null,
    }
}

/// A value shaped like the worst case for every leaf: a URL carrying a password after a username.
pub fn canary(index: usize) -> String {
    format!("user:S3CRETcanary{index}@canary{index}.example")
}

/// Runs `render` over a document per leaf and returns the leaves whose canary shows in its
/// output although the schema does not mark them secret and `allowed` does not name them.
///
/// `allowed` is the list of leaves a person has judged not to be secrets: a path, a user name, a
/// host. A leaf in neither list is a string nobody has classified, which is the mistake the check
/// exists for: a new `String` option holding a token is such a leaf.
pub fn unclassified_or_leaking(
    schema: &Value,
    allowed: &[&str],
    render: impl Fn(Value) -> Result<String, String>,
) -> Vec<String> {
    let mut problems = Vec::new();
    for (index, leaf) in string_leaves(schema).iter().enumerate() {
        let text = canary(index);
        let document = document_setting(schema, leaf, &text);
        let output = match render(document) {
            Ok(output) => output,
            Err(error) => {
                problems.push(format!("{}: the document was refused: {error}", leaf.path));
                continue;
            }
        };
        let secret_shown = output.contains(&format!("S3CRETcanary{index}@"))
            || output.contains(&format!("S3CRETcanary{index}\""));
        let user_shown = output.contains(&text);
        if leaf.write_only {
            if secret_shown || user_shown {
                problems.push(format!("{}: a secret is logged in plain text", leaf.path));
            }
        } else if !allowed.contains(&leaf.path.as_str()) && (secret_shown || user_shown) {
            problems.push(format!(
                "{}: a string option that is logged and is neither marked secret in the schema nor allowed as plain",
                leaf.path
            ));
        }
    }
    problems
}

/// A document, as the effective configuration's log would carry it.
pub fn render_channel_defaults(document: Value) -> Result<String, String> {
    let options: ChannelOptions = serde_json::from_value(document).map_err(|error| error.to_string())?;
    let effective = Effective {
        memory_ceiling: None,
        memory_hard_ceiling: None,
        channel_defaults: Some(options),
    };
    Ok(format!("{effective:?}"))
}
