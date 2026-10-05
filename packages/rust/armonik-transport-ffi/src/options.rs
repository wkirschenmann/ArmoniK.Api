//! The runtime's options as a vocabulary: the schema a host generates its options class from.
//!
//! `ak_runtime_create` takes them as `ak_runtime_config`, where zero asks for the default. A host
//! reads them from its configuration instead, where an option left out is the default, so the
//! schema names the same fields and refuses the zero a configuration has no reason to write.

/// What a caller may set on the runtime.
#[derive(Default, schemars::JsonSchema)]
#[schemars(rename_all = "PascalCase", deny_unknown_fields)]
pub struct RuntimeOptions {
    /// The bytes the runtime holds before work waits, counting the buffers lent to send and the
    /// messages received until the host gives them back: a call stops reading, and a send waits
    /// for room.
    ///
    /// Defaults to 4294967295, four gigabytes, or half the address space where that is smaller;
    /// a larger value is that too.
    #[schemars(with = "i64", range(min = 1), default)]
    pub memory_ceiling: Option<u64>,

    /// The bytes past which the runtime stops: a received message that would take the count past
    /// them ends its call with RESOURCE_EXHAUSTED. Calls admitted to read below MemoryCeiling may
    /// pass it together, by a message each, and this bounds them. At least MemoryCeiling, or
    /// MemoryCeiling's default when that is left out.
    ///
    /// Defaults to a quarter above MemoryCeiling.
    #[schemars(with = "i64", range(min = 1), default)]
    pub memory_hard_ceiling: Option<u64>,

    /// Channel options every channel of the runtime takes where its own options state none: the
    /// two are merged option by option, a struct's options within it, and the channel's win; an
    /// alternative - how the server is verified, who the client is, which proxy - merges its fields
    /// over the same alternative and is taken whole over another.
    ///
    /// Defaults to none.
    #[schemars(with = "armonik_transport::options::ChannelOptions", default)]
    pub channel_defaults: Option<armonik_transport::options::ChannelOptions>,
}

/// The JSON schema of [`RuntimeOptions`], as committed beside this crate.
pub fn schema() -> String {
    // Every option is optional, and the default each takes is its description's to state.
    let schema = schemars::generate::SchemaSettings::default()
        .with_transform(schemars::transform::RecursiveTransform(
            |schema: &mut schemars::Schema| {
                schema.remove("default");
            },
        ))
        .into_generator()
        .into_root_schema_for::<RuntimeOptions>();
    let mut rendered = serde_json::to_string_pretty(&schema).expect("a schema renders");
    rendered.push('\n');
    rendered
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The file the C# generator reads is the one these types describe.
    #[test]
    fn the_committed_schema_is_the_one_the_types_describe() {
        let committed = include_str!("../runtime.schema.json").replace("\r\n", "\n");

        assert_eq!(
            committed,
            schema(),
            "the options changed and the schema did not; write it again with\n  \
             cargo run -p armonik-transport-ffi --features schema --example runtime_schema -- \
             packages/rust/armonik-transport-ffi/runtime.schema.json"
        );
    }

    /// Every option the schema declares is a field of `ak_runtime_config`, under the name a
    /// binding maps it to.
    #[test]
    fn every_option_is_a_field_of_the_config() {
        let schema: serde_json::Value =
            serde_json::from_str(&schema()).expect("the schema is a document");
        let mut names: Vec<_> = schema["properties"]
            .as_object()
            .expect("the schema has properties")
            .keys()
            .cloned()
            .collect();
        names.sort();

        let config = crate::abi::ak_runtime_config {
            struct_size: 0,
            version: 0,
            flags: 0,
            reserved: 0,
            memory_ceiling: 0,
            memory_hard_ceiling: 0,
            channel_defaults_json: crate::abi::ak_bytes_in {
                ptr: std::ptr::null(),
                len: 0,
            },
        };
        let _ = (
            config.memory_ceiling,
            config.memory_hard_ceiling,
            config.channel_defaults_json,
        );
        assert_eq!(
            names,
            ["ChannelDefaults", "MemoryCeiling", "MemoryHardCeiling"]
        );
    }
}
