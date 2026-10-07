//! `ak_runtime_create_from` against the configuration fixtures the loader is held to, and what
//! it refuses of the structure itself before any source is read.

// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

// The refusal this reads is the loader's whole message, so the parts the loader's own tests read
// apart are not read here.
#[allow(dead_code)]
#[path = "../../armonik-transport/tests/common/configuration.rs"]
mod fixtures;

use armonik_transport::configuration::Configuration;
use armonik_transport::options::RuntimeOptions;
use armonik_transport_ffi::*;
use fixtures::{Fixture, Outcome, Source, Staged};
use support::host::{send_one, start_call, Host, Refused};
use support::{TestServer, ECHO};

fn view(bytes: &[u8]) -> ak_bytes_in {
    ak_bytes_in {
        ptr: bytes.as_ptr(),
        len: bytes.len(),
    }
}

fn source(kind: ak_source_kind, value: &[u8]) -> ak_config_source {
    ak_config_source {
        kind: kind as u32,
        reserved: 0,
        value: view(value),
    }
}

fn config(sources: &[ak_config_source], prefix: &[u8], flags: u32) -> ak_config {
    ak_config {
        struct_size: std::mem::size_of::<ak_config>() as u32,
        version: 0,
        flags,
        source_count: sources.len() as u32,
        sources: sources.as_ptr(),
        prefix: view(prefix),
    }
}

/// A fixture's sources as the loader takes them and as the ABI does, from the same staged files.
struct Both {
    values: Vec<(ak_source_kind, Vec<u8>)>,
    configuration: Configuration,
}

fn both(fixture: &Fixture, staged: &Staged) -> Both {
    let mut configuration = match &fixture.prefix {
        None => Configuration::new(),
        Some(prefix) => Configuration::with_prefix(prefix),
    };
    let mut values = Vec::new();
    for source in &fixture.sources {
        let (kind, value) = match source {
            Source::File(name) => {
                let path = staged.path(name);
                configuration = configuration.file(&path);
                (
                    ak_source_kind::AK_SOURCE_FILE,
                    path.to_string_lossy().into_owned(),
                )
            }
            Source::OptionalFile(name) => {
                let path = staged.path(name);
                configuration = configuration.optional_file(&path);
                (
                    ak_source_kind::AK_SOURCE_OPTIONAL_FILE,
                    path.to_string_lossy().into_owned(),
                )
            }
            Source::Environment => {
                configuration = configuration.environment();
                (ak_source_kind::AK_SOURCE_ENVIRONMENT, String::new())
            }
            Source::Pairs(pairs) => {
                configuration = configuration.pairs(pairs.clone());
                let object: serde_json::Map<String, serde_json::Value> = pairs
                    .iter()
                    .map(|(key, value)| (key.clone(), serde_json::Value::String(value.clone())))
                    .collect();
                (
                    ak_source_kind::AK_SOURCE_PAIRS,
                    serde_json::Value::Object(object).to_string(),
                )
            }
            Source::PairsJson(json) => {
                configuration = configuration.pairs_json(json.clone());
                (ak_source_kind::AK_SOURCE_PAIRS, json.clone())
            }
            Source::Document(json) => {
                configuration = configuration.document(json.clone());
                (ak_source_kind::AK_SOURCE_DOCUMENT, json.clone())
            }
        };
        values.push((kind, value.into_bytes()));
    }
    Both {
        values,
        configuration,
    }
}

/// The same sources give the same options, or the same refusal, through the loader and through
/// the ABI. One test, the fixtures one after the other: they set the process's environment, and a
/// process holds one runtime at a time.
#[test]
fn every_fixture_creates_the_runtime_or_earns_the_refusal_the_loader_gives() {
    for (index, fixture) in fixtures::fixtures().iter().enumerate() {
        let staged = Staged::new(fixture, "abi", index);
        let Both {
            values,
            configuration,
        } = both(fixture, &staged);
        let sources: Vec<ak_config_source> = values
            .iter()
            .map(|(kind, value)| source(*kind, value))
            .collect();
        let (prefix, flags) = match fixture.prefix.as_deref() {
            None => ("", 0),
            Some("") => ("", AK_CONFIG_NO_PREFIX),
            Some(prefix) => (prefix, 0),
        };
        let loaded = configuration.load::<RuntimeOptions>();
        let created = Host::from_config(&config(&sources, prefix.as_bytes(), flags));
        let name = &fixture.name;

        match (&fixture.outcome, loaded, created) {
            (Outcome::Options(expected), Ok(loaded), Ok(host)) => {
                let expected: RuntimeOptions =
                    serde_json::from_value(expected.clone()).expect("the fixture's options");
                assert_eq!(loaded, expected, "{name}");
                assert_eq!(
                    hooks::runtime_options(host.runtime),
                    Some(expected),
                    "{name}"
                );
            }
            (Outcome::Refused { .. }, Err(loaded), Err(refused)) => {
                assert_eq!(refused.status, ak_status::AK_STATUS_INVALID_ARG, "{name}");
                assert_eq!(refused.kind, ak_error_kind::AK_ERROR_CONFIG, "{name}");
                assert_eq!(refused.detail, loaded.to_string(), "{name}");
            }
            (_, loaded, created) => panic!(
                "{name}: the loader gives {loaded:?}, the ABI {}",
                match created {
                    Ok(_) => "a runtime".to_owned(),
                    Err(refused) => format!("{refused:?}"),
                }
            ),
        }
    }
}

/// What is malformed in the structure is refused as a misuse of the ABI, and before any source
/// is read: each configuration below lists first a file that does not exist, whose refusal would
/// be a configuration's.
#[test]
fn a_malformed_structure_is_refused_before_any_source_is_read() {
    let absent = source(ak_source_kind::AK_SOURCE_FILE, b"no/such/file.json");
    let document = source(ak_source_kind::AK_SOURCE_DOCUMENT, b"{}");
    let unnamed = ak_config_source {
        kind: 6,
        ..document
    };
    let zero = ak_config_source {
        kind: 0,
        ..document
    };
    let reserved = ak_config_source {
        reserved: 1,
        ..document
    };
    let environment_with_a_value = source(ak_source_kind::AK_SOURCE_ENVIRONMENT, b"x");
    let not_utf8 = source(ak_source_kind::AK_SOURCE_DOCUMENT, &[0xff, 0xfe]);
    let null_view = ak_config_source {
        value: ak_bytes_in {
            ptr: std::ptr::null(),
            len: 2,
        },
        ..document
    };

    let malformed: Vec<(&str, Vec<ak_config_source>, &[u8], u32)> = vec![
        ("a kind past the last", vec![absent, unnamed], b"", 0),
        ("a kind of zero", vec![absent, zero], b"", 0),
        ("a reserved field set", vec![absent, reserved], b"", 0),
        (
            "an environment source with a value",
            vec![absent, environment_with_a_value],
            b"",
            0,
        ),
        ("a value that is not UTF-8", vec![absent, not_utf8], b"", 0),
        ("a null view with a length", vec![absent, null_view], b"", 0),
        (
            "a prefix beside AK_CONFIG_NO_PREFIX",
            vec![absent],
            b"GrpcClient",
            AK_CONFIG_NO_PREFIX,
        ),
        ("a flag no one defines", vec![absent], b"", 2),
        ("a prefix that is not UTF-8", vec![absent], &[0xff], 0),
    ];
    for (what, sources, prefix, flags) in malformed {
        let Err(refused) = Host::from_config(&config(&sources, prefix, flags)) else {
            panic!("{what} is admitted");
        };
        assert_eq!(refused.status, ak_status::AK_STATUS_INVALID_ARG, "{what}");
        assert_eq!(
            refused.kind,
            ak_error_kind::AK_ERROR_USAGE,
            "{what}: {refused:?}"
        );
    }

    let sources = [absent];
    let mut versioned = config(&sources, b"", 0);
    versioned.version = 1;
    let mut short = config(&sources, b"", 0);
    short.struct_size -= 1;
    let mut no_sources = config(&sources, b"", 0);
    no_sources.sources = std::ptr::null();
    for (what, malformed) in [
        ("a version past zero", versioned),
        ("a record shorter than the first definition", short),
        ("a count of sources and no pointer to them", no_sources),
    ] {
        let Err(refused) = Host::from_config(&malformed) else {
            panic!("{what} is admitted");
        };
        assert_eq!(refused.status, ak_status::AK_STATUS_INVALID_ARG, "{what}");
        assert_eq!(
            refused.kind,
            ak_error_kind::AK_ERROR_USAGE,
            "{what}: {refused:?}"
        );
    }
}

/// A runtime's Endpoint is what a channel created with an empty one reaches, and a runtime with
/// none refuses such a channel.
#[test]
fn an_empty_endpoint_is_the_runtimes() {
    let server = TestServer::start();
    let document = format!(r#"{{"Endpoint":"{}"}}"#, server.endpoint);
    let sources = [source(
        ak_source_kind::AK_SOURCE_DOCUMENT,
        document.as_bytes(),
    )];
    let host = Host::from_config(&config(&sources, b"", 0)).expect("a runtime");

    let channel = host.channel("");
    let call = start_call(channel, ECHO, &[]);
    send_one(call, b"hello");
    let seen = host.recorder.await_terminal();
    assert_eq!(seen.message_payloads(), vec![b"hello".to_vec()]);
    assert_eq!(seen.status_code(), Some(0));
    ak_channel_release(channel);
    drop(host);

    let host = Host::start();
    let refused: Refused = host
        .try_channel("", "{}")
        .expect_err("no endpoint to reach");
    assert_eq!(refused.status, ak_status::AK_STATUS_INVALID_ARG);
    assert_eq!(refused.kind, ak_error_kind::AK_ERROR_CONFIG);
    assert!(refused.detail.contains("Endpoint"), "{refused:?}");
}

/// The options a document states and the ABI refuses although the loader admits them: a zero
/// ceiling, which a configuration leaves out for the default, and an endpoint that is not a URI,
/// which is not quoted.
#[test]
fn the_runtime_refuses_what_it_could_not_be_created_with() {
    for (document, says, never) in [
        (r#"{"MemoryCeiling":0}"#, "MemoryCeiling", None),
        (r#"{"MemoryHardCeiling":0}"#, "MemoryHardCeiling", None),
        (r#"{"Endpoint":""}"#, "Endpoint", None),
        (
            r#"{"Endpoint":"http://alice:s3cret@ not a uri"}"#,
            "Endpoint",
            Some("s3cret"),
        ),
        (
            r#"{"ChannelDefaults":{"Grpc":{"Host":{"Receive":{"Window":0}}}}}"#,
            "ChannelDefaults",
            None,
        ),
    ] {
        let sources = [source(
            ak_source_kind::AK_SOURCE_DOCUMENT,
            document.as_bytes(),
        )];
        let Err(refused) = Host::from_config(&config(&sources, b"", 0)) else {
            panic!("{document} is admitted");
        };
        assert_eq!(
            refused.status,
            ak_status::AK_STATUS_INVALID_ARG,
            "{document}"
        );
        assert_eq!(refused.kind, ak_error_kind::AK_ERROR_CONFIG, "{document}");
        assert!(refused.detail.contains(says), "{refused:?}");
        if let Some(never) = never {
            assert!(!refused.detail.contains(never), "{refused:?}");
        }
    }
}
