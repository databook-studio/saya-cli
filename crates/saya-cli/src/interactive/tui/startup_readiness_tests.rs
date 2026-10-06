//! `StartupReadiness::from_runtime` over hand-built runtimes: the splash's
//! AI line must read the same facts `config doctor --task ai` reports —
//! ready with the provider detail, not ready with the first failing
//! check's detail — and never carry a secret value. The fixtures build a
//! `RuntimeConfig` from TOML the way `doctor_task_tests.rs` does; no
//! network, no file touched beyond the resolver's in-memory maps.

use std::collections::BTreeMap;

use super::StartupReadiness;
use crate::config::runtime::RuntimeConfig;

fn runtime_from(config_toml: &str, secrets: BTreeMap<String, String>) -> RuntimeConfig {
    use saya_config::{ConfigFile, ConnectionsFile, ResolutionInput, resolve};
    let config = ConfigFile::from_toml(config_toml).unwrap();
    let connections = ConnectionsFile::from_toml("").unwrap();
    let resolved = resolve(ResolutionInput::new(connections.clone()).with_user(config)).unwrap();
    RuntimeConfig {
        resolved,
        connections,
        config_path: None,
        connections_path: None,
        cache_scope: std::path::PathBuf::from("/tmp/saya-startup-readiness-test"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: secrets,
    }
}

/// A local provider is ready, and the detail is the provider check's own
/// "ollama, model <m>" line.
#[test]
fn an_ollama_config_reads_ready_with_the_provider_detail() {
    let runtime = runtime_from(
        "[ai]\nprovider = \"ollama\"\nmodel = \"qwen2.5-coder:14b\"\n",
        BTreeMap::new(),
    );
    let startup = StartupReadiness::from_runtime(&runtime);
    assert!(startup.ai.ready, "{:?}", startup.ai);
    assert_eq!(startup.ai.detail, "ollama, model qwen2.5-coder:14b");
}

/// A cloud provider without a key reference is not ready, and the detail
/// is the credentials check — the thing actually missing, not the
/// provider line that already passed.
#[test]
fn a_cloud_provider_without_a_key_reads_not_ready_with_the_credential_detail() {
    let runtime = runtime_from(
        "[ai]\nprovider = \"openai\"\nmodel = \"gpt-4o\"\n",
        BTreeMap::new(),
    );
    let startup = StartupReadiness::from_runtime(&runtime);
    assert!(!startup.ai.ready, "{:?}", startup.ai);
    assert_eq!(
        startup.ai.detail,
        "no api_key reference configured — requests to openai will be unauthenticated"
    );
}

/// A configured provider with no model fails first: the detail names the
/// model gap, which is the first failing check.
#[test]
fn a_missing_model_reads_not_ready_with_the_provider_detail() {
    let runtime = runtime_from(
        "[ai]\nprovider = \"ollama\"\nmodel = \"\"\n",
        BTreeMap::new(),
    );
    let startup = StartupReadiness::from_runtime(&runtime);
    assert!(!startup.ai.ready, "{:?}", startup.ai);
    assert_eq!(startup.ai.detail, "no model is configured — set [ai] model");
}

/// The detail never carries a secret value: the sentinel sits in
/// `secret_values`, the reference resolves, and only the provider detail —
/// never the key — reaches the screen.
#[test]
fn the_detail_never_carries_a_secret_value() {
    const SENTINEL: &str = "sentinel-ai-key-never-print-this";
    let runtime = runtime_from(
        "[ai]\nprovider = \"openai\"\nmodel = \"gpt-4o\"\napi_key = { env = \"SAYA_TEST_AI_KEY\" }\n",
        BTreeMap::from([("SAYA_TEST_AI_KEY".to_string(), SENTINEL.to_string())]),
    );
    let startup = StartupReadiness::from_runtime(&runtime);
    assert!(startup.ai.ready, "{:?}", startup.ai);
    assert_eq!(startup.ai.detail, "openai, model gpt-4o");
    assert!(
        !startup.ai.detail.contains(SENTINEL),
        "a secret value reached the startup detail: {:?}",
        startup.ai.detail
    );
}
