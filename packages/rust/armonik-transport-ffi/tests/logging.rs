//! What a host receives of the engine's logs: through the log callback given at creation, filtered
//! by the runtime's `Logging.Filter`, with the load's events delivered on the creating thread and
//! no secret in the effective configuration.

// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::thread::ThreadId;

use armonik_transport::options::ChannelOptions;
use armonik_transport_ffi::*;
use serde_json::{json, Map, Value};
use support::host::{send_one, start_call, Host, Refused};
use support::{TestServer, ECHO};

#[derive(Clone, Debug)]
struct Record {
    level: u32,
    target: String,
    message: String,
    fields: Vec<(String, String)>,
    thread: ThreadId,
}

impl Record {
    fn field(&self, key: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    fn text(&self) -> String {
        format!("{self:?}")
    }
}

static LOGS: Mutex<Vec<Record>> = Mutex::new(Vec::new());
static CALLS: AtomicUsize = AtomicUsize::new(0);
/// Taken by each test for its whole length: the log is the process's, as the runtime is.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
static CONTEXT: u8 = 0;

fn context() -> *mut c_void {
    std::ptr::addr_of!(CONTEXT).cast_mut().cast()
}

unsafe fn text(view: ak_bytes_in) -> String {
    if view.len == 0 {
        return String::new();
    }
    let bytes = unsafe { std::slice::from_raw_parts(view.ptr, view.len) };
    std::str::from_utf8(bytes)
        .expect("a record is UTF-8")
        .to_owned()
}

unsafe extern "C" fn collect(log_ctx: *mut c_void, record: *const ak_log_record) {
    assert_eq!(log_ctx, context(), "the context the host gave comes back");
    let record = unsafe { &*record };
    assert_eq!(
        record.struct_size as usize,
        std::mem::size_of::<ak_log_record>()
    );
    let fields = if record.field_count == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(record.fields, record.field_count) }
            .iter()
            .map(|field| unsafe { (text(field.key), text(field.value)) })
            .collect()
    };
    let kept = Record {
        level: record.level,
        target: unsafe { text(record.target) },
        message: unsafe { text(record.message) },
        fields,
        thread: std::thread::current().id(),
    };
    CALLS.fetch_add(1, Ordering::SeqCst);
    LOGS.lock()
        .unwrap_or_else(|held| held.into_inner())
        .push(kept);
}

fn turn() -> MutexGuard<'static, ()> {
    let turn = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|held| held.into_inner());
    LOGS.lock().unwrap_or_else(|held| held.into_inner()).clear();
    turn
}

fn logged() -> Vec<Record> {
    LOGS.lock().unwrap_or_else(|held| held.into_inner()).clone()
}

fn bytes(text: &str) -> ak_bytes_in {
    ak_bytes_in {
        ptr: text.as_ptr(),
        len: text.len(),
    }
}

/// A runtime created from one document, its logs going to `collect`.
fn create(document: &str) -> Result<Host, Refused> {
    let sources = [ak_config_source {
        kind: ak_source_kind::AK_SOURCE_DOCUMENT as u32,
        reserved: 0,
        value: bytes(document),
    }];
    Host::from_config(&ak_config {
        struct_size: std::mem::size_of::<ak_config>() as u32,
        version: 0,
        flags: 0,
        source_count: 1,
        sources: sources.as_ptr(),
        prefix: bytes(""),
        log_callback: Some(collect),
        log_ctx: context(),
    })
}

fn with_filter(filter: &str) -> String {
    json!({ "Logging": { "Filter": filter } }).to_string()
}

fn has(records: &[Record], target: &str, level: u32, message: &str) -> bool {
    records.iter().any(|record| {
        record.target.starts_with(target)
            && record.level == level
            && record.message.contains(message)
    })
}

#[test]
fn an_unknown_key_is_logged_at_info_on_the_thread_that_creates_the_runtime() {
    let _turn = turn();
    let host = create(r#"{"Misspelled":1,"Elsewhere":{"Level":"debug"}}"#).expect("a runtime");

    let records = logged();
    let unknown: Vec<_> = records
        .iter()
        .filter(|record| record.message.contains("does not know"))
        .collect();
    assert_eq!(unknown.len(), 2, "{records:#?}");
    for record in &unknown {
        assert_eq!(record.level, AK_LOG_INFO);
        assert_eq!(record.thread, std::thread::current().id());
        assert_eq!(record.field("source"), Some("a document"));
    }
    let keys: Vec<_> = unknown
        .iter()
        .filter_map(|record| record.field("key"))
        .collect();
    assert!(keys.contains(&"Misspelled"), "{keys:?}");
    assert!(keys.contains(&"Elsewhere"), "{keys:?}");
    // A value is never quoted.
    assert!(records
        .iter()
        .all(|record| !record.text().contains("\"1\"")));
    drop(host);
}

#[test]
fn the_runtimes_effective_configuration_is_logged_once_it_is_created() {
    let _turn = turn();
    let host = create(
        r#"{"MemoryCeiling":{"SoftMiB":1},"ChannelDefaults":{"Http2":{"SimultaneousCallsPerConnection":{"Limit":4}}}}"#,
    )
    .expect("a runtime");

    let records = logged();
    let effective: Vec<_> = records
        .iter()
        .filter(|record| record.message.contains("runtime's effective configuration"))
        .collect();
    assert_eq!(effective.len(), 1, "{records:#?}");
    let record = effective[0];
    assert_eq!(record.level, AK_LOG_INFO);
    assert_eq!(record.field("memory_ceiling"), Some("1048576"));
    assert!(
        record
            .field("channel_defaults")
            .is_some_and(|defaults| defaults.contains("Limit(4)")),
        "{record:#?}"
    );
    assert_eq!(
        record.field("log_filter"),
        Some("*=warn,armonik_transport*=info")
    );
    drop(host);
}

/// A filter the user gives replaces the default whole, and a target none of its directives covers
/// is off: a word that is no level is a target nothing emits, so it shows nothing, and
/// `armonik_transport=debug` alone shows that target alone.
#[test]
fn a_stated_filter_replaces_the_default_and_what_it_does_not_cover_is_off() {
    let _turn = turn();
    let host = create(&with_filter("Information")).expect("a runtime");
    tracing::error!(target: "h2::test", "a library at error");
    tracing::error!(target: "armonik_transport::test", "the engine at error");
    drop(host);
    assert!(logged().is_empty(), "{:#?}", logged());

    let host = create(&with_filter("armonik_transport=debug")).expect("a runtime");
    tracing::debug!(target: "armonik_transport::test", "the named target");
    tracing::error!(target: "armonik_transport_ffi::test", "the same text, not its module");
    tracing::error!(target: "h2::test", "a library at error");
    drop(host);
    let messages: Vec<_> = logged().into_iter().map(|record| record.message).collect();
    assert_eq!(messages, ["the named target"]);
}

/// With no filter the default applies: warnings from every target, the engine's own at info.
#[test]
fn with_no_filter_the_default_applies() {
    let _turn = turn();
    let host = create("{}").expect("a runtime");
    tracing::info!(target: "armonik_transport_ffi::test", "the engine at info");
    tracing::debug!(target: "armonik_transport_ffi::test", "the engine at debug");
    tracing::warn!(target: "h2::test", "a library at warn");
    tracing::info!(target: "h2::test", "a library at info");
    drop(host);
    let messages: Vec<_> = logged()
        .into_iter()
        .filter(|record| record.target.ends_with("::test"))
        .map(|record| record.message)
        .collect();
    assert_eq!(messages, ["the engine at info", "a library at warn"]);
}

/// A filter that states a level for every target says what the rest is: `*=off` alone is no logs at
/// all, and `*=off` with a target is that target alone.
#[test]
fn a_star_in_the_filter_says_what_it_does_not_cover() {
    let _turn = turn();
    let host = create(&with_filter("*=off")).expect("a runtime");
    tracing::error!(target: "h2::test", "silenced");
    tracing::error!(target: "armonik_transport::test", "silenced too");
    drop(host);
    assert!(logged().is_empty(), "{:#?}", logged());

    let host = create(&with_filter("*=off,armonik_transport=debug")).expect("a runtime");
    tracing::debug!(target: "armonik_transport::test", "the named target");
    tracing::error!(target: "armonik_transport_ffi::test", "not named");
    tracing::error!(target: "h2::test", "another library");
    drop(host);
    let messages: Vec<_> = logged().into_iter().map(|record| record.message).collect();
    assert_eq!(messages, ["the named target"]);
}

#[test]
fn a_channel_that_states_options_logs_them_with_its_endpoint_and_one_that_states_none_does_not() {
    let _turn = turn();
    let host = create("{}").expect("a runtime");
    LOGS.lock().unwrap_or_else(|held| held.into_inner()).clear();

    let plain = host.channel_with("http://127.0.0.1:1", "{}");
    assert!(
        logged()
            .iter()
            .all(|record| !record.message.contains("channel's effective")),
        "{:#?}",
        logged()
    );
    ak_channel_release(plain);

    // An endpoint with credentials is refused, after its options are logged without them.
    let refused = host.try_channel(
        "http://user:hunter2@127.0.0.1:1",
        r#"{"Http2":{"SimultaneousCallsPerConnection":{"Limit":7}}}"#,
    );
    assert!(refused.is_err());
    let records = logged();
    let effective: Vec<_> = records
        .iter()
        .filter(|record| record.message.contains("channel's effective"))
        .collect();
    assert_eq!(effective.len(), 1, "{records:#?}");
    assert_eq!(effective[0].field("endpoint"), Some("http://127.0.0.1:1"));
    assert!(effective[0]
        .field("options")
        .is_some_and(|options| options.contains("Limit(7)")));
    assert!(records
        .iter()
        .all(|record| !record.text().contains("hunter2")));
    drop(host);
}

/// A channel that states nothing takes the runtime's defaults, which the runtime's log has said.
#[test]
fn a_channel_that_states_nothing_over_runtime_defaults_logs_nothing_of_its_own() {
    let _turn = turn();
    let host =
        create(r#"{"ChannelDefaults":{"Http2":{"SimultaneousCallsPerConnection":{"Limit":4}}}}"#)
            .expect("a runtime");
    LOGS.lock().unwrap_or_else(|held| held.into_inner()).clear();

    let inherited = host.channel_with("http://127.0.0.1:1", "{}");
    assert!(
        logged()
            .iter()
            .all(|record| !record.message.contains("channel's effective")),
        "{:#?}",
        logged()
    );
    ak_channel_release(inherited);

    let own = host.channel_with(
        "http://127.0.0.1:1",
        r#"{"Http2":{"SimultaneousCallsPerConnection":{"Limit":5}}}"#,
    );
    let records = logged();
    let effective: Vec<_> = records
        .iter()
        .filter(|record| record.message.contains("channel's effective"))
        .collect();
    assert_eq!(effective.len(), 1, "{records:#?}");
    assert!(effective[0]
        .field("options")
        .is_some_and(|options| options.contains("Limit(5)")));
    ak_channel_release(own);
    drop(host);
}

#[test]
fn the_filter_option_selects_what_the_engine_logs_and_the_default_keeps_debug_out() {
    let _turn = turn();
    let server = TestServer::start();

    let host = create("{}").expect("a runtime");
    let channel = host.channel(&server.endpoint);
    ak_channel_release(channel);
    drop(host);
    assert!(
        logged().iter().all(|record| record.level <= AK_LOG_INFO),
        "debug under the default filter: {:#?}",
        logged()
    );

    // The callsites the first runtime found disabled are asked again.
    LOGS.lock().unwrap_or_else(|held| held.into_inner()).clear();
    let host = create(&with_filter("armonik_transport=debug")).expect("a runtime");
    let channel = host.channel(&server.endpoint);
    let call = start_call(channel, ECHO, &[]);
    send_one(call, b"hello");
    host.recorder.await_terminal();
    ak_channel_release(channel);
    drop(host);

    let records = logged();
    assert!(
        has(
            &records,
            "armonik_transport::grpc::channel",
            AK_LOG_DEBUG,
            "channel created"
        ),
        "{records:#?}"
    );
    let dialled = records
        .iter()
        .find(|record| record.message == "dialling")
        .expect("the dial is logged");
    assert_ne!(
        dialled.thread,
        std::thread::current().id(),
        "the dial runs on the channel's thread"
    );
    assert!(has(
        &records,
        "armonik_transport::grpc::channel",
        AK_LOG_DEBUG,
        "session opened"
    ));
    // The filter was loaded with the rest of the configuration: the load's own events obey it.
    assert!(records.iter().all(|record| record.level != AK_LOG_TRACE));
}

#[test]
fn a_directive_that_is_not_understood_is_logged_as_ignored_and_the_rest_is_held() {
    let _turn = turn();
    let host = create(&with_filter("h2=loud,armonik_transport_ffi=warn")).expect("a runtime");
    let records = logged();
    assert!(
        records
            .iter()
            .any(|record| record.level == AK_LOG_WARN
                && record.field("directive") == Some("h2=loud")),
        "{records:#?}"
    );
    // The directive that holds replaces the default: the engine's info events are off.
    tracing::info!(target: "armonik_transport::test", "the engine at info");
    let records = logged();
    assert!(
        records
            .iter()
            .all(|record| !record.message.contains("runtime's effective")
                && record.message != "the engine at info"),
        "{records:#?}"
    );
    drop(host);
}

/// The warning about a directive is not subject to the filter it is about: this one selects nothing
/// but h2, and still says what it ignored.
#[test]
fn an_ignored_directive_is_reported_whatever_the_filter_selects() {
    let _turn = turn();
    let host = create(&with_filter("h2=debug,foo=loud")).expect("a runtime");
    let records = logged();
    assert!(
        records
            .iter()
            .any(|record| record.level == AK_LOG_WARN
                && record.field("directive") == Some("foo=loud")),
        "{records:#?}"
    );
    drop(host);
}

#[test]
fn a_refused_creation_still_delivers_what_its_load_logged_and_nothing_after() {
    let _turn = turn();
    let Err(refused) = create(r#"{"Unknown":1,"MemoryCeiling":{"SoftMiB":"many"}}"#) else {
        panic!("a ceiling that is not a number is admitted");
    };
    assert_eq!(refused.status, ak_status::AK_STATUS_INVALID_ARG);
    let before = CALLS.load(Ordering::SeqCst);
    assert!(
        logged()
            .iter()
            .any(|record| record.message.contains("does not know")),
        "{:#?}",
        logged()
    );

    // The callback is gone with the refusal: nothing reaches it, and a host may free its context.
    tracing::info!(target: "armonik_transport::test", "after the refusal");
    assert_eq!(CALLS.load(Ordering::SeqCst), before);
}

#[test]
fn nothing_is_delivered_once_the_runtime_is_destroyed() {
    let _turn = turn();
    let host = create(&with_filter("trace")).expect("a runtime");
    tracing::info!(target: "armonik_transport::test", "while it lives");
    assert!(logged()
        .iter()
        .any(|record| record.message == "while it lives"));
    drop(host);

    let before = CALLS.load(Ordering::SeqCst);
    tracing::error!(target: "armonik_transport::test", "after it was destroyed");
    assert_eq!(CALLS.load(Ordering::SeqCst), before);
}

#[test]
fn a_host_that_gives_no_callback_logs_nothing_and_a_short_record_gives_none() {
    let _turn = turn();
    // A record whose size ends before the log fields: they read as none.
    let sources: [ak_config_source; 0] = [];
    let host = Host::from_config(&ak_config {
        struct_size: std::mem::offset_of!(ak_config, log_callback) as u32,
        version: 0,
        flags: 0,
        source_count: 0,
        sources: sources.as_ptr(),
        prefix: bytes(""),
        // Past the host's record, which this library reads as zero.
        log_callback: Some(collect),
        log_ctx: context(),
    })
    .expect("a runtime");
    tracing::error!(target: "armonik_transport::test", "nobody listens");
    assert!(logged().is_empty(), "{:#?}", logged());
    drop(host);
}

/// `ak_runtime_config` has no field for the filter, which only the loader's options give: a
/// runtime created from it logs by the default.
#[test]
fn ak_runtime_create_logs_by_the_default_filter() {
    let _turn = turn();
    let config = ak_runtime_config {
        struct_size: std::mem::size_of::<ak_runtime_config>() as u32,
        version: 0,
        flags: 0,
        reserved: 0,
        memory_ceiling: 0,
        memory_hard_ceiling: 0,
        channel_defaults_json: bytes(r#"{"Unknown":1}"#),
        log_callback: Some(collect),
        log_ctx: context(),
    };
    let host = Host::from_runtime_config(&config);
    let records = logged();
    assert!(
        records
            .iter()
            .any(|record| record.message.contains("does not know")),
        "{records:#?}"
    );

    tracing::info!(target: "armonik_transport::test", "the engine at info");
    tracing::debug!(target: "armonik_transport::test", "the engine at debug");
    tracing::info!(target: "h2::test", "a library at info");
    tracing::warn!(target: "h2::test", "a library at warn");
    let messages: Vec<_> = logged()
        .into_iter()
        .filter(|record| record.target.ends_with("::test"))
        .map(|record| record.message)
        .collect();
    assert_eq!(messages, ["the engine at info", "a library at warn"]);
    drop(host);
}

/// A target of `*` alone is every target, so that a filter can bring everything back.
#[test]
fn a_star_alone_selects_every_target() {
    let _turn = turn();
    let host = create(&with_filter("*=debug")).expect("a runtime");
    tracing::debug!(target: "anything::at_all", "reported");
    tracing::trace!(target: "anything::at_all", "not reported");
    let messages: Vec<_> = logged()
        .into_iter()
        .filter(|record| record.target == "anything::at_all")
        .map(|record| record.message)
        .collect();
    assert_eq!(messages, ["reported"]);
    drop(host);
}

/// An event logged from inside the callback is dropped, as the header says: delivered, it would
/// enter the callback again on its own stack.
#[test]
fn an_event_logged_from_inside_the_callback_is_dropped() {
    static INNER: AtomicUsize = AtomicUsize::new(0);

    unsafe extern "C" fn reentrant(log_ctx: *mut c_void, record: *const ak_log_record) {
        unsafe { collect(log_ctx, record) };
        INNER.fetch_add(1, Ordering::SeqCst);
        tracing::info!(target: "armonik_transport::test", "from inside");
    }

    let _turn = turn();
    INNER.store(0, Ordering::SeqCst);
    let sources: [ak_config_source; 0] = [];
    let host = Host::from_config(&ak_config {
        struct_size: std::mem::size_of::<ak_config>() as u32,
        version: 0,
        flags: 0,
        source_count: 0,
        sources: sources.as_ptr(),
        prefix: bytes(""),
        log_callback: Some(reentrant),
        log_ctx: context(),
    })
    .expect("a runtime");
    let delivered = INNER.load(Ordering::SeqCst);
    assert!(delivered >= 1, "the runtime logged its configuration");
    assert!(logged()
        .iter()
        .all(|record| record.message != "from inside"));
    drop(host);
}

/// One host thread per delivery: records of different threads arrive as they happen.
#[test]
fn records_reach_the_callback_from_every_thread_that_logs() {
    let _turn = turn();
    let host = create(&with_filter("armonik_transport=info")).expect("a runtime");
    let threads: Vec<_> = (0..4)
        .map(|index| {
            std::thread::spawn(move || {
                for step in 0..50 {
                    tracing::info!(target: "armonik_transport::test", index, step, "from a thread");
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().expect("a thread");
    }
    let from_threads = logged()
        .into_iter()
        .filter(|record| record.message == "from a thread")
        .count();
    assert_eq!(from_threads, 200);
    drop(host);
}

// What a secret is, by the schema: the string options of a channel, followed through `$ref` and
// every alternative, and whether the schema marks each one `writeOnly`.

struct Leaf {
    path: String,
    write_only: bool,
}

fn schema() -> Value {
    serde_json::from_str(&armonik_transport::options::schema()).expect("a schema")
}

fn resolve<'a>(root: &'a Value, node: &'a Value) -> &'a Value {
    match node.get("$ref").and_then(Value::as_str) {
        Some(reference) => &root["$defs"][reference.rsplit('/').next().unwrap_or_default()],
        None => node,
    }
}

fn leaves(root: &Value, node: &Value, path: &str, secret: bool, out: &mut Vec<Leaf>, depth: usize) {
    assert!(depth < 16, "the schema nests deeper than this walk follows");
    let secret = secret || node.get("writeOnly").and_then(Value::as_bool) == Some(true);
    let node = resolve(root, node);
    let secret = secret || node.get("writeOnly").and_then(Value::as_bool) == Some(true);
    if let Some(alternatives) = node.get("oneOf").and_then(Value::as_array) {
        for alternative in alternatives {
            leaves(root, alternative, path, secret, out, depth + 1);
        }
        return;
    }
    if let Some(properties) = node.get("properties").and_then(Value::as_object) {
        for (name, child) in properties {
            let at = if path.is_empty() {
                name.clone()
            } else {
                format!("{path}.{name}")
            };
            leaves(root, child, &at, secret, out, depth + 1);
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
            write_only: secret,
        });
    }
}

/// The smallest document that sets the leaf at `path`: the path itself, and beside each object on
/// the way what it requires.
fn document(root: &Value, node: &Value, path: &[&str], value: &str, depth: usize) -> Value {
    let node = resolve(root, node);
    if path.is_empty() {
        return Value::String(value.to_owned());
    }
    if let Some(alternatives) = node.get("oneOf").and_then(Value::as_array) {
        let chosen = alternatives
            .iter()
            .find(|alternative| alternative["properties"].get(path[0]).is_some())
            .unwrap_or(&alternatives[0]);
        return document(root, chosen, path, value, depth + 1);
    }
    let properties = node.get("properties").and_then(Value::as_object);
    let mut object = Map::new();
    if let Some(child) = properties.and_then(|properties| properties.get(path[0])) {
        object.insert(
            path[0].to_owned(),
            document(root, child, &path[1..], value, depth + 1),
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
    if depth > 16 {
        return Value::Null;
    }
    if let Some(alternatives) = node.get("oneOf").and_then(Value::as_array) {
        return placeholder(root, &alternatives[0], depth + 1);
    }
    if let Some(value) = node.get("const") {
        return value.clone();
    }
    if let Some(properties) = node.get("properties").and_then(Value::as_object) {
        let mut object = Map::new();
        for name in node
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if let Some(child) = properties.get(name) {
                object.insert(name.to_owned(), placeholder(root, child, depth + 1));
            }
        }
        return Value::Object(object);
    }
    match node.get("type").and_then(Value::as_str) {
        Some("string") => Value::String("placeholder".to_owned()),
        Some("boolean") => Value::Bool(false),
        Some("integer" | "number") => node
            .get("minimum")
            .cloned()
            .unwrap_or_else(|| Value::from(1)),
        _ => Value::Null,
    }
}

/// The string options a person has judged not to be secrets: a path, a name, a host. A string
/// option in neither this list nor marked `writeOnly` by the schema is one nobody has classified,
/// which is what an option added later as plain text looks like.
const NOT_SECRET: &[&str] = &[
    "Grpc.UserAgent",
    "Transport.Proxy.System.Username",
    "Transport.Proxy.Url.Username",
    "Transport.Tls.ClientCertificate.P12.Path",
    "Transport.Tls.ClientCertificate.Pem.Certificate",
    "Transport.Tls.ClientCertificate.Pem.Key",
    "Transport.Tls.ClientCertificate.Store.Find.FriendlyName",
    "Transport.Tls.ClientCertificate.Store.Find.SubjectName",
    "Transport.Tls.ClientCertificate.Store.Find.Thumbprint",
    "Transport.Tls.ClientCertificate.Store.Name",
    "Transport.Tls.ServerCertificates.CaPem",
    "Transport.Tls.ServerCertificates.CaStore.Find.FriendlyName",
    "Transport.Tls.ServerCertificates.CaStore.Find.SubjectName",
    "Transport.Tls.ServerCertificates.CaStore.Find.Thumbprint",
    "Transport.Tls.ServerCertificates.CaStore.Name",
];

/// The canary of the leaf `index`: a URL shaped like the worst case, a password after a user name.
fn canary(index: usize) -> (String, String) {
    (format!("canaryuser{index}"), format!("S3CRETcanary{index}"))
}

/// Sets each string option of the channel's schema in turn - as a channel's own document, and as
/// the runtime's channel defaults - and finds what the effective configuration's log says of it:
/// nothing for a secret, and for one that is not, only if a person said it may be shown.
#[test]
fn no_secret_option_is_logged_and_no_string_option_is_logged_unclassified() {
    let _turn = turn();
    let root = schema();
    let mut all = Vec::new();
    leaves(&root, &root, "", false, &mut all, 0);
    all.sort_by(|a, b| a.path.cmp(&b.path));
    all.dedup_by(|a, b| a.path == b.path);
    assert!(
        all.iter().any(|leaf| leaf.write_only),
        "the schema marks no option secret, so the walk finds none"
    );

    let mut problems = Vec::new();
    let mut unreached = Vec::new();
    let mut shown_plain = 0;
    for (index, leaf) in all.iter().enumerate() {
        let (user, secret) = canary(index);
        let value = format!("{user}:{secret}@canary{index}.example");
        let segments: Vec<&str> = leaf.path.split('.').collect();
        let channel = document(&root, &root, &segments, &value, 0);
        let mut reached = false;

        // As a channel's own document, over a runtime that has no defaults.
        let host = create("{}").expect("a runtime");
        LOGS.lock().unwrap_or_else(|held| held.into_inner()).clear();
        let made = host.try_channel("http://127.0.0.1:1", &channel.to_string());
        let own = logged();
        if let Ok(made) = made {
            ak_channel_release(made);
        }
        drop(host);

        // As the runtime's defaults.
        let defaults = json!({ "ChannelDefaults": channel }).to_string();
        let defaulted = match create(&defaults) {
            Ok(host) => {
                drop(host);
                logged()
            }
            Err(_) => Vec::new(),
        };

        for (how, records) in [
            ("a channel's own", own),
            ("the runtime's defaults", defaulted),
        ] {
            let shown = records.iter().any(|record| {
                let text = record.text();
                text.contains(&secret) || text.contains(&user)
            });
            let logged_at_all = records
                .iter()
                .any(|record| record.message.contains("effective configuration"));
            if leaf.write_only {
                if shown {
                    problems.push(format!("{} as {how}: a secret is logged", leaf.path));
                }
                reached |= logged_at_all;
            } else if shown && NOT_SECRET.contains(&leaf.path.as_str()) {
                shown_plain += 1;
            } else if shown {
                problems.push(format!(
                    "{} as {how}: a string option is logged, and is neither marked secret by the schema nor classified as plain here",
                    leaf.path
                ));
            }
        }
        // A document the engine refuses before it logs - a bundle that is not on disk, a proxy
        // address that carries credentials - is checked as the log renders the options.
        if leaf.write_only && !reached {
            if let Ok(options) = serde_json::from_value::<ChannelOptions>(channel.clone()) {
                let rendered = format!("{options:?}");
                if rendered.contains(&secret) || rendered.contains(&user) {
                    problems.push(format!("{}: a secret is rendered", leaf.path));
                }
                reached = true;
            }
        }
        if leaf.write_only && !reached {
            unreached.push(leaf.path.clone());
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
    // A value the log is allowed to show is seen, so a canary that never reached it is noticed.
    assert!(
        shown_plain > 0,
        "no allowed string option was found in a log"
    );
    // Each secret was set in a document the engine took and logged, or the check saw nothing it
    // could have caught for that option.
    assert!(
        unreached.is_empty(),
        "no log of the configuration was made with these secrets set: {unreached:#?}"
    );
}

/// The check above finds a string option nobody classified.
#[test]
fn an_unclassified_string_option_is_what_the_check_would_catch() {
    let _turn = turn();
    let schema = json!({
        "type": "object",
        "properties": {
            "UserAgent": { "type": "string" },
            "ApiToken": { "type": "string" }
        }
    });
    let mut all = Vec::new();
    leaves(&schema, &schema, "", false, &mut all, 0);
    let paths: Vec<_> = all.iter().map(|leaf| leaf.path.as_str()).collect();
    assert_eq!(paths, ["UserAgent", "ApiToken"]);
    assert!(all.iter().all(|leaf| !leaf.write_only));
}

/// The endpoint's credentials never reach the log of the runtime's configuration.
#[test]
fn the_runtimes_endpoint_is_logged_without_its_credentials() {
    let _turn = turn();
    let host = create(r#"{"Endpoint":"http://alice:s3cret@127.0.0.1:1"}"#).expect("a runtime");
    let records = logged();
    assert!(
        records
            .iter()
            .all(|record| !record.text().contains("s3cret")),
        "{records:#?}"
    );
    assert!(
        records
            .iter()
            .all(|record| !record.text().contains("alice")),
        "{records:#?}"
    );
    assert!(
        records
            .iter()
            .any(|record| record.field("endpoint") == Some("http://127.0.0.1:1")),
        "{records:#?}"
    );
    drop(host);
}
