use std::path::PathBuf;

use saya_harness::endpoints::{CorpusProfile, EndpointSpec};
use saya_types::SecretRef;

#[path = "run_configs/secrets.rs"]
mod secrets;

#[path = "run_configs/bindings.rs"]
mod bindings;

#[path = "run_configs/connections.rs"]
mod connections;

#[path = "run_configs/permissions.rs"]
mod permissions;

#[path = "run_configs/regeneration.rs"]
mod regeneration;

fn temp_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "saya-harness-run-configs-{label}-{}",
        std::process::id()
    ))
}

fn endpoint(name: &str, model: &str, api_key: Option<SecretRef>) -> EndpointSpec {
    EndpointSpec {
        name: name.to_string(),
        provider: "anthropic".to_string(),
        model: model.to_string(),
        base_url: Some("https://api.test".to_string()),
        api_key,
    }
}

fn corpus(name: &str, path: &str) -> CorpusProfile {
    CorpusProfile {
        name: name.to_string(),
        path: path.to_string(),
    }
}
