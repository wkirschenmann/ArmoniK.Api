//! Point 7: every option rendered by its own type, and a check that finds a plain-text secret.

use observability_spike::config_log::{
    canary, document_setting, log_effective, log_unknown_key, render_channel_defaults, string_leaves,
    unclassified_or_leaking, Effective,
};
use observability_spike::obs::RtObs;
use observability_spike::runtime::{Front, ObsRuntime};
use observability_spike::testkit::{collect, Collector};
use serde_json::{json, Value};

/// The string options a person has judged not to be secrets: paths, names, hosts.
const NOT_SECRET: &[&str] = &[
    "Grpc.UserAgent",
    "Transport.Proxy.System.Username",
    "Transport.Proxy.Url.Username",
    "Transport.Tls.Client.P12.Path",
    "Transport.Tls.Client.Pem.Certificate",
    "Transport.Tls.Client.Pem.Key",
    "Transport.Tls.Client.Store.Find.FriendlyName",
    "Transport.Tls.Client.Store.Find.SubjectName",
    "Transport.Tls.Client.Store.Find.Thumbprint",
    "Transport.Tls.Client.Store.Name",
    "Transport.Tls.OverrideTargetName",
    "Transport.Tls.Server.CaPem",
    "Transport.Tls.Server.CaStore.Find.FriendlyName",
    "Transport.Tls.Server.CaStore.Find.SubjectName",
    "Transport.Tls.Server.CaStore.Find.Thumbprint",
    "Transport.Tls.Server.CaStore.Name",
];

fn schema() -> Value {
    serde_json::from_str(&armonik_transport::options::schema()).unwrap()
}

#[test]
fn list_the_string_leaves() {
    for leaf in string_leaves(&schema()) {
        println!("{:60} write_only={}", leaf.path, leaf.write_only);
    }
}

/// Every leaf of the real schema is classified: marked secret and never shown, or allowed.
#[test]
fn no_string_option_of_the_channel_options_is_shown_in_plain_unless_it_is_allowed() {
    let problems = unclassified_or_leaking(&schema(), NOT_SECRET, render_channel_defaults);
    assert!(problems.is_empty(), "{problems:#?}");
}

/// The same check over a type that renders a secret in plain: it finds it.
#[test]
fn a_new_string_option_holding_a_token_is_found() {
    #[derive(Debug, serde::Deserialize)]
    #[serde(rename_all = "PascalCase")]
    #[allow(dead_code)]
    struct Imagined {
        user_agent: Option<String>,
        api_token: Option<String>,
    }
    let schema = json!({
        "type": "object",
        "properties": {
            "UserAgent": { "type": "string" },
            "ApiToken": { "type": "string" }
        }
    });
    let render = |document: Value| -> Result<String, String> {
        let options: Imagined = serde_json::from_value(document).map_err(|e| e.to_string())?;
        Ok(format!("{options:?}"))
    };
    // A person classified UserAgent; nobody classified ApiToken.
    let problems = unclassified_or_leaking(&schema, &["UserAgent"], render);
    assert_eq!(problems.len(), 1, "{problems:#?}");
    assert!(problems[0].starts_with("ApiToken"), "{problems:#?}");
    let _ = (canary(0), document_setting);
    // Drop the helper reference noise: the harness is what the test is about.
    let _ = NOT_SECRET;
}

/// The log event itself, through the record a host receives, carries no secret.
#[test]
fn the_logged_event_holds_no_secret() {
    let runtime = ObsRuntime::new(Front::Layered);
    let collector = Box::new(Collector::default());
    let obs: &RtObs = &runtime.obs;
    obs.begin_load();
    let _inside = runtime.scope();

    let options: armonik_transport::options::ChannelOptions = serde_json::from_value(json!({
        "Transport": {
            "Proxy": { "UrlWithCredentials": "http://proxyuser:proxypassword@proxy.example:3128" },
            "ConnectTimeoutSeconds": 3.5
        },
        "Tls": null
    }))
    .or_else(|_| {
        serde_json::from_value(json!({
            "Transport": {
                "Proxy": { "UrlWithCredentials": "http://proxyuser:proxypassword@proxy.example:3128" },
                "ConnectTimeoutSeconds": 3.5
            }
        }))
    })
    .unwrap();
    log_effective(&Effective {
        memory_ceiling: Some(1 << 30),
        memory_hard_ceiling: None,
        channel_defaults: Some(options),
    });
    log_unknown_key("channel_defaults_json", "Transport.Proxx");
    obs.end_load();
    obs.set_log_callback(collect, collector.ctx(), "info").unwrap();

    let kept = collector.take();
    for record in &kept {
        println!("{} {} fields={:?}", record.target, record.message, record.fields);
    }
    assert_eq!(kept.len(), 2, "{kept:#?}");
    let text = format!("{kept:?}");
    assert!(!text.contains("proxypassword"), "{text}");
    assert!(!text.contains("proxyuser"), "{text}");
    assert!(text.contains("proxy.example"), "{text}");
}
