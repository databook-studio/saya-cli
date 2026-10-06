//! Unit tests for `config show --explain`: the winning layer of every
//! user-facing setting, read back through the same merge funnel that resolved
//! the value. The invariants under test: every key is a user-facing key in
//! table order, the layer names the variable/flag for env/CLI sources, file
//! layers name the path a user would edit, the ignored project overrides
//! carry the pinned reason, and no secret value ever appears — in JSON or in
//! the text rendering.

use std::collections::BTreeMap;
use std::path::PathBuf;

use super::{ConfigExplanation, IgnoredOverride, SettingExplanation, explain, text};
use crate::config::runtime::RuntimeConfig;
use saya_config::{
    CliOverrides, ConfigFile, ConnectionsFile, ResolutionInput, SettingSource, resolve,
};

const CONNECTIONS: &str = "[profiles.local]\ntype = 'duckdb'\npath = ':memory:'\n";

/// Hand-builds a `RuntimeConfig` the way the loader does: `resolve` runs the
/// real merge funnel, so the provenance recorded here is the one the CLI
/// would display. `config_path` mirrors the loader's selection (the project
/// file when it exists, `--config` when passed).
fn runtime_from(
    user_toml: &str,
    project_toml: &str,
    cli: CliOverrides,
    process_env: BTreeMap<String, String>,
    config_path: Option<PathBuf>,
) -> RuntimeConfig {
    let connections = ConnectionsFile::from_toml(CONNECTIONS).unwrap();
    let resolved = resolve(
        ResolutionInput::new(connections.clone())
            .with_user(ConfigFile::from_toml(user_toml).unwrap())
            .with_project(ConfigFile::from_toml(project_toml).unwrap())
            .with_process_env(process_env.clone())
            .with_cli(cli),
    )
    .unwrap();
    RuntimeConfig {
        resolved,
        connections,
        config_path,
        connections_path: None,
        cache_scope: PathBuf::from("/tmp/saya-explain-test"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        // The loader seeds secret_values from the process environment; the
        // fixture mirrors that, so a leak of an env-sourced VALUE is visible
        // to the sentinel test below.
        secret_values: process_env,
    }
}

fn entry<'a>(explanation: &'a ConfigExplanation, key: &str) -> &'a SettingExplanation {
    explanation
        .settings
        .iter()
        .find(|setting| setting.key == key)
        .unwrap_or_else(|| panic!("no `{key}` setting in {explanation:?}"))
}

// --- an empty configuration is all defaults ---

#[test]
fn a_default_config_reports_every_setting_as_built_in_default() {
    let runtime = runtime_from("", "", CliOverrides::default(), BTreeMap::new(), None);
    let explanation = explain(&runtime);
    assert!(
        explanation.ignored_project_overrides.is_empty(),
        "{explanation:?}"
    );
    assert_eq!(
        explanation.settings.len(),
        saya_config::user_facing_keys().len(),
        "every user-facing setting is explained"
    );
    for setting in &explanation.settings {
        assert_eq!(setting.layer, SettingSource::Default, "{:?}", setting.key);
        assert_eq!(setting.source, "built-in default", "{:?}", setting.key);
        assert_eq!(setting.file, None, "{:?}", setting.key);
        assert_eq!(setting.reference, None, "{:?}", setting.key);
        // Drift guard for the diagnostics-field → dotted-key table: every
        // explained key is one the merge funnels actually recorded, so a
        // mistyped key cannot hide behind the Default fallback.
        assert!(
            runtime.resolved.provenance.get(&setting.key).is_some(),
            "no origin recorded for {:?}",
            setting.key
        );
    }
}

#[test]
fn every_key_is_a_user_facing_key_in_table_order() {
    let runtime = runtime_from("", "", CliOverrides::default(), BTreeMap::new(), None);
    let explanation = explain(&runtime);
    for (setting, (_, key)) in explanation
        .settings
        .iter()
        .zip(saya_config::user_facing_keys())
    {
        assert_eq!(setting.key, *key, "order follows the user-facing table");
    }
}

// --- file layers name the file a user would edit ---

#[test]
fn a_user_file_model_is_attributed_to_the_user_config_file() {
    let runtime = runtime_from(
        "[ai]\nmodel = 'user-model'\n",
        "",
        CliOverrides::default(),
        BTreeMap::new(),
        None,
    );
    let explanation = explain(&runtime);
    let model = entry(&explanation, "ai.model");
    assert_eq!(model.layer, SettingSource::UserFile);
    assert_eq!(model.source, "user config file");
    assert_eq!(
        model.file.as_deref(),
        Some(
            crate::config::sources::user_config_dir()
                .join("config.toml")
                .to_str()
                .unwrap()
        )
    );
    assert_eq!(model.value, serde_json::json!("user-model"));
}

#[test]
fn a_project_file_model_is_attributed_to_the_project_config_file() {
    let project = PathBuf::from("/saya-test/project/.saya/config.toml");
    let runtime = runtime_from(
        "",
        "[ai]\nmodel = 'project-model'\n",
        CliOverrides::default(),
        BTreeMap::new(),
        Some(project.clone()),
    );
    let explanation = explain(&runtime);
    let model = entry(&explanation, "ai.model");
    assert_eq!(model.layer, SettingSource::ProjectFile);
    assert_eq!(model.source, "project config file");
    assert_eq!(model.file.as_deref(), Some(project.to_str().unwrap()));
    assert_eq!(model.value, serde_json::json!("project-model"));
}

// --- env and CLI sources name the variable or flag, never a file ---

#[test]
fn an_env_var_source_names_the_variable_and_never_a_file() {
    let env = BTreeMap::from([("SAYA_MODEL".to_string(), "env-model".to_string())]);
    let runtime = runtime_from("", "", CliOverrides::default(), env, None);
    let explanation = explain(&runtime);
    let model = entry(&explanation, "ai.model");
    assert_eq!(
        model.layer,
        SettingSource::Env {
            var: "SAYA_MODEL".into()
        }
    );
    assert_eq!(model.source, "environment (SAYA_MODEL)");
    assert_eq!(model.file, None);
    assert_eq!(model.value, serde_json::json!("env-model"));
}

#[test]
fn a_cli_flag_source_names_the_flag_and_never_a_file() {
    let cli = CliOverrides {
        model: Some("cli-model".into()),
        ..Default::default()
    };
    let runtime = runtime_from("", "", cli, BTreeMap::new(), None);
    let explanation = explain(&runtime);
    let model = entry(&explanation, "ai.model");
    assert_eq!(
        model.layer,
        SettingSource::Cli {
            flag: "--model".into()
        }
    );
    assert_eq!(model.source, "command line (--model)");
    assert_eq!(model.file, None);
}

// --- the ignored project overrides carry the pinned reason ---

#[test]
fn an_ignored_project_override_is_listed_with_its_reason_and_the_users_value_wins() {
    let runtime = runtime_from(
        "[ai]\nbase_url = 'https://user.example/v1'\n",
        "[ai]\nbase_url = 'https://project.example/v1'\n",
        CliOverrides::default(),
        BTreeMap::new(),
        Some(PathBuf::from("/saya-test/project/.saya/config.toml")),
    );
    let explanation = explain(&runtime);
    let ignored = &explanation.ignored_project_overrides;
    assert_eq!(ignored.len(), 1, "{explanation:?}");
    assert_eq!(ignored[0].key, "ai.base_url");
    assert_eq!(
        ignored[0].reason,
        "the project file cannot set ai.base_url: it is security-critical; \
         set it in your user config file or pass --trust-project-config"
    );
    let base_url = entry(&explanation, "ai.base_url");
    assert_eq!(base_url.layer, SettingSource::UserFile);
    let value = base_url.value.as_str().unwrap();
    assert!(value.contains("user.example"), "{value}");
    assert!(!value.contains("project.example"), "{value}");
}

// --- a secret reference shows only its redacted label ---

#[test]
fn a_secret_reference_shows_only_its_redacted_label_and_never_a_value() {
    const SENTINEL: &str = "sentinel-explain-key-never-print";
    let env = BTreeMap::from([("SAYA_AI_API_KEY".to_string(), SENTINEL.to_string())]);
    let runtime = runtime_from(
        "[ai]\nmodel = 'm'\n",
        "",
        CliOverrides::default(),
        env,
        None,
    );
    let explanation = explain(&runtime);
    let api_key = entry(&explanation, "ai.api_key");
    assert_eq!(api_key.value, serde_json::json!("env:SAYA_AI_API_KEY"));
    assert_eq!(api_key.reference.as_deref(), Some("env:SAYA_AI_API_KEY"));
    let serialized = serde_json::to_string(&explanation).unwrap();
    assert!(!serialized.contains(SENTINEL), "{serialized}");
    let rendered = text(&explanation);
    assert!(!rendered.contains(SENTINEL), "{rendered}");
}

// --- the wire shape ---

#[test]
fn the_serialized_shape_is_settings_plus_ignored_project_overrides() {
    let runtime = runtime_from("", "", CliOverrides::default(), BTreeMap::new(), None);
    let explanation = explain(&runtime);
    let value: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&explanation).unwrap()).unwrap();
    let settings = value["settings"].as_array().unwrap();
    assert!(value["ignored_project_overrides"].is_array());
    assert_eq!(settings[0]["key"], "profile_name");
    assert_eq!(settings[0]["source"], "built-in default");
    assert_eq!(settings[0]["layer"]["layer"], "default");
    assert!(settings[0]["value"].is_string());
    assert!(settings[0].get("file").is_none(), "{:?}", settings[0]);
    assert!(settings[0].get("reference").is_none(), "{:?}", settings[0]);
}

// --- the text rendering ---

#[test]
fn the_text_rendering_pins_one_aligned_line_per_setting() {
    let explanation = ConfigExplanation {
        settings: vec![
            SettingExplanation {
                key: "ai.model".into(),
                value: serde_json::json!("test-model"),
                source: "user config file".into(),
                layer: SettingSource::UserFile,
                file: Some("/u/c/config.toml".into()),
                reference: None,
            },
            SettingExplanation {
                key: "run.max_rows".into(),
                value: serde_json::json!(50),
                source: "built-in default".into(),
                layer: SettingSource::Default,
                file: None,
                reference: None,
            },
            SettingExplanation {
                key: "ai.api_key".into(),
                value: serde_json::json!("env:SAYA_AI_API_KEY"),
                source: "environment (SAYA_AI_API_KEY)".into(),
                layer: SettingSource::Env {
                    var: "SAYA_AI_API_KEY".into(),
                },
                file: None,
                reference: Some("env:SAYA_AI_API_KEY".into()),
            },
        ],
        ignored_project_overrides: vec![],
    };
    assert_eq!(
        text(&explanation),
        format!(
            "{:<38} {}  ← {}\n{:<38} {}  ← {}\n{:<38} {}  ← {}",
            "ai.model",
            "test-model",
            "user config file (/u/c/config.toml)",
            "run.max_rows",
            "50",
            "built-in default",
            "ai.api_key",
            "env:SAYA_AI_API_KEY",
            "environment (SAYA_AI_API_KEY)",
        )
    );
}

#[test]
fn the_text_rendering_lists_ignored_project_overrides_after_a_blank_line() {
    let explanation = ConfigExplanation {
        settings: vec![SettingExplanation {
            key: "ai.base_url".into(),
            value: serde_json::json!("https://user.example/v1"),
            source: "user config file".into(),
            layer: SettingSource::UserFile,
            file: Some("/u/c/config.toml".into()),
            reference: None,
        }],
        ignored_project_overrides: vec![IgnoredOverride {
            key: "ai.base_url".into(),
            reason: "the project file cannot set ai.base_url: it is security-critical; \
                     set it in your user config file or pass --trust-project-config"
                .into(),
        }],
    };
    assert_eq!(
        text(&explanation),
        format!(
            "{:<38} {}  ← {}\n\nIgnored from the project file:\n  {}",
            "ai.base_url",
            "https://user.example/v1",
            "user config file (/u/c/config.toml)",
            "the project file cannot set ai.base_url: it is security-critical; \
             set it in your user config file or pass --trust-project-config",
        )
    );
}
