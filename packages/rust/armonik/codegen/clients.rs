//! The clients of the services, generated from the descriptor set prost-build writes: one per
//! service, over the engine's channel, each method calling the helper of its call's shape.
//!
//! Read from the descriptor rather than plugged into prost-build, so that what it needs of the
//! protos - services, methods, their types and whether each side streams - is all it depends on.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

use prost::Message;
use prost_types::{DescriptorProto, FileDescriptorSet};

/// Appends to each package's generated file the clients of the services it declares.
pub fn generate(descriptor_set: &Path, out_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let set = FileDescriptorSet::decode(std::fs::read(descriptor_set)?.as_slice())?;

    let mut types = HashMap::new();
    for file in &set.file {
        let package = file.package();
        for message in &file.message_type {
            name_messages(package, &[], message, &mut types);
        }
    }

    let mut clients = HashMap::<&str, String>::new();
    for file in &set.file {
        let package = file.package();
        for service in &file.service {
            let code = clients.entry(package).or_default();
            client(package, service, &types, code)?;
        }
    }

    for (package, code) in clients {
        let path = out_dir.join(format!("{package}.rs"));
        let mut generated = std::fs::read_to_string(&path)?;
        generated.push_str(&code);
        std::fs::write(&path, generated)?;
    }
    Ok(())
}

/// Where a message lives: its package, and the messages it is nested in, outermost first.
struct Location {
    package: String,
    nesting: Vec<String>,
}

fn name_messages(
    package: &str,
    outer: &[String],
    message: &DescriptorProto,
    types: &mut HashMap<String, Location>,
) {
    let mut nesting = outer.to_vec();
    nesting.push(message.name().to_owned());
    for nested in &message.nested_type {
        name_messages(package, &nesting, nested, types);
    }
    types.insert(
        format!(".{package}.{}", nesting.join(".")),
        Location {
            package: package.to_owned(),
            nesting,
        },
    );
}

/// The Rust path of message `name`, from inside a client module of `package`.
fn rust_type(
    package: &str,
    name: &str,
    types: &HashMap<String, Location>,
) -> Result<String, String> {
    if let Some(well_known) = name.strip_prefix(".google.protobuf.") {
        // As prost-build names them: the wrappers and Empty are Rust types, the rest prost-types'.
        let rust = match well_known {
            "Empty" => "()",
            "BoolValue" => "bool",
            "BytesValue" => "::prost::alloc::vec::Vec<u8>",
            "DoubleValue" => "f64",
            "FloatValue" => "f32",
            "Int32Value" => "i32",
            "Int64Value" => "i64",
            "StringValue" => "::prost::alloc::string::String",
            "UInt32Value" => "u32",
            "UInt64Value" => "u64",
            _ => return Ok(format!("::prost_types::{well_known}")),
        };
        return Ok(rust.to_owned());
    }
    let location = types
        .get(name)
        .ok_or_else(|| format!("`{name}` is no message of the protos"))?;
    let from: Vec<&str> = package.split('.').collect();
    let to: Vec<&str> = location.package.split('.').collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();

    // One `super` out of the client's own module, then one per package level above the common one.
    let mut path = "super::".repeat(1 + from.len() - common);
    for segment in &to[common..] {
        write!(path, "{}::", snake(segment)).expect("writing to a String");
    }
    let (message, outer) = location.nesting.split_last().expect("a message has a name");
    for parent in outer {
        write!(path, "{}::", snake(parent)).expect("writing to a String");
    }
    path.push_str(message);
    Ok(path)
}

fn client(
    package: &str,
    service: &prost_types::ServiceDescriptorProto,
    types: &HashMap<String, Location>,
    code: &mut String,
) -> Result<(), String> {
    let name = service.name();
    let module = format!("{}_client", snake(name));
    writeln!(
        code,
        r#"
/// The client of `{package}.{name}`, over the engine's channel.
#[cfg(feature = "_gen-client")]
pub mod {module} {{
    #[derive(Clone, Debug)]
    pub struct {name}Client {{
        channel: ::armonik_transport::grpc::GrpcChannel,
    }}

    impl {name}Client {{
        pub fn new(channel: ::armonik_transport::grpc::GrpcChannel) -> Self {{
            Self {{ channel }}
        }}"#
    )
    .expect("writing to a String");

    for method in &service.method {
        let path = format!("/{package}.{name}/{}", method.name());
        let input = rust_type(package, method.input_type(), types)?;
        let output = rust_type(package, method.output_type(), types)?;
        let function = snake(method.name());
        let (parameter, helper, answer) =
            match (method.client_streaming(), method.server_streaming()) {
                (false, false) => ("request: impl Into<{input}>", "unary", "{output}"),
                (false, true) => (
                    "request: impl Into<{input}>",
                    "server_streaming",
                    "crate::client::rpc::Streaming<{output}>",
                ),
                (true, false) => ("requests: S", "client_streaming", "{output}"),
                (true, true) => (
                    "requests: S",
                    "bidi_streaming",
                    "crate::client::rpc::Streaming<{output}>",
                ),
            };
        let parameter = parameter.replace("{input}", &input);
        let answer = answer.replace("{output}", &output);
        let (generics, bound, argument) = if method.client_streaming() {
            (
                "<S>",
                format!("\n        where\n            S: ::futures::Stream<Item = {input}> + Send + 'static,"),
                "requests",
            )
        } else {
            ("", String::new(), "request.into()")
        };
        writeln!(
            code,
            r#"
        pub async fn {function}{generics}(
            &self,
            {parameter},
        ) -> Result<crate::client::rpc::Response<{answer}>, ::armonik_transport::grpc::GrpcStatus>{bound}
        {{
            crate::client::rpc::{helper}(&self.channel, "{path}", {argument}).await
        }}"#
        )
        .expect("writing to a String");
    }
    writeln!(code, "    }}\n}}").expect("writing to a String");
    Ok(())
}

/// `name` in snake case, as prost and tonic write a module or a method: a word starts at an
/// upper-case letter that follows a lower-case letter or a digit, or that a lower-case letter
/// follows after another upper-case one.
fn snake(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut snake = String::new();
    for (at, &c) in chars.iter().enumerate() {
        if c.is_uppercase() {
            let after_lower =
                at > 0 && (chars[at - 1].is_lowercase() || chars[at - 1].is_ascii_digit());
            let ends_acronym = at > 0
                && chars[at - 1].is_uppercase()
                && chars.get(at + 1).is_some_and(|next| next.is_lowercase());
            if after_lower || ends_acronym {
                snake.push('_');
            }
            snake.extend(c.to_lowercase());
        } else {
            snake.push(c);
        }
    }
    snake
}
