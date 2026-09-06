//! DRAFT: prints the JSON schema of the channel options.
fn main() {
    let schema = schemars::schema_for!(armonik_transport::options::ChannelOptions);
    println!(
        "{}",
        serde_json::to_string_pretty(&schema).expect("a schema")
    );
}
