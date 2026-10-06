//! Setting provenance: which layer supplied each user-facing setting.
//!
//! Provenance is a value-free observation of the existing merge. It names
//! the winning layer and — for a secret — only the redacted reference
//! label, never a value or a path, and it never changes resolution: the
//! same values, the same errors, the same `ignored_project_overrides`.

use saya_config::{
    CliOverrides, ConfigFile, ConnectionsFile, ResolutionInput, SettingSource, resolve,
};

fn empty() -> ResolutionInput {
    ResolutionInput::new(ConnectionsFile::default())
}

fn source_of(resolved: &saya_config::ResolvedConfig, key: &str) -> SettingSource {
    resolved
        .provenance
        .get(key)
        .unwrap_or_else(|| panic!("no provenance recorded for {key}"))
        .source
        .clone()
}

#[test]
fn provenance_names_the_winning_layer_across_the_precedence_chain() {
    // User file only.
    let resolved =
        resolve(empty().with_user(ConfigFile::from_toml("[ai]\nmodel = 'user'\n").unwrap()))
            .unwrap();
    assert_eq!(resolved.ai.model, "user");
    let origin = resolved.provenance.get("ai.model").unwrap();
    assert_eq!(origin.source, SettingSource::UserFile);
    assert_eq!(origin.reference, None);

    // A project file overlays it.
    let resolved = resolve(
        empty()
            .with_user(ConfigFile::from_toml("[ai]\nmodel = 'user'\n").unwrap())
            .with_project(ConfigFile::from_toml("[ai]\nmodel = 'project'\n").unwrap()),
    )
    .unwrap();
    assert_eq!(resolved.ai.model, "project");
    assert_eq!(source_of(&resolved, "ai.model"), SettingSource::ProjectFile);

    // An env file overlays the project layer.
    let resolved = resolve(
        empty()
            .with_user(ConfigFile::from_toml("[ai]\nmodel = 'user'\n").unwrap())
            .with_project(ConfigFile::from_toml("[ai]\nmodel = 'project'\n").unwrap())
            .with_env_file([("SAYA_AI_MODEL", "env-file")]),
    )
    .unwrap();
    assert_eq!(resolved.ai.model, "env-file");
    assert_eq!(
        source_of(&resolved, "ai.model"),
        SettingSource::EnvFile {
            var: "SAYA_AI_MODEL".into()
        }
    );

    // The process environment overlays the env file; the SAYA_MODEL alias
    // records the variable actually used.
    let resolved = resolve(
        empty()
            .with_user(ConfigFile::from_toml("[ai]\nmodel = 'user'\n").unwrap())
            .with_project(ConfigFile::from_toml("[ai]\nmodel = 'project'\n").unwrap())
            .with_env_file([("SAYA_AI_MODEL", "env-file")])
            .with_process_env([("SAYA_MODEL", "process")]),
    )
    .unwrap();
    assert_eq!(resolved.ai.model, "process");
    assert_eq!(
        source_of(&resolved, "ai.model"),
        SettingSource::Env {
            var: "SAYA_MODEL".into()
        }
    );

    // The command line wins.
    let resolved = resolve(
        empty()
            .with_user(ConfigFile::from_toml("[ai]\nmodel = 'user'\n").unwrap())
            .with_project(ConfigFile::from_toml("[ai]\nmodel = 'project'\n").unwrap())
            .with_env_file([("SAYA_AI_MODEL", "env-file")])
            .with_process_env([("SAYA_MODEL", "process")])
            .with_cli(CliOverrides {
                model: Some("cli".into()),
                ..Default::default()
            }),
    )
    .unwrap();
    assert_eq!(resolved.ai.model, "cli");
    assert_eq!(
        source_of(&resolved, "ai.model"),
        SettingSource::Cli {
            flag: "--model".into()
        }
    );

    // Nothing at all: the built-in default, both in value and in provenance.
    let resolved = resolve(empty()).unwrap();
    assert_eq!(resolved.ai.model, "qwen2.5-coder:14b");
    assert_eq!(source_of(&resolved, "ai.model"), SettingSource::Default);
}

#[test]
fn an_ignored_project_override_keeps_the_trusted_provenance() {
    let user = ConfigFile::from_toml("[ai]\nbase_url = 'https://user.test/v1'\n").unwrap();
    let project = ConfigFile::from_toml("[ai]\nbase_url = 'https://project.test/v1'\n").unwrap();
    let resolved = resolve(
        empty()
            .with_user(user.clone())
            .with_project(project.clone()),
    )
    .unwrap();
    assert_eq!(
        resolved.ai.base_url.as_deref(),
        Some("https://user.test/v1")
    );
    let origin = resolved.provenance.get("ai.base_url").unwrap();
    assert_eq!(origin.source, SettingSource::UserFile);
    assert_eq!(origin.reference, None);
    assert!(
        resolved
            .ignored_project_overrides
            .iter()
            .any(|key| key == "ai.base_url")
    );

    // With the project layer trusted it wins, and is attributed to it.
    let resolved = resolve(
        empty()
            .with_user(user)
            .with_project(project)
            .with_cli(CliOverrides {
                trust_project_config: true,
                ..Default::default()
            }),
    )
    .unwrap();
    assert_eq!(
        resolved.ai.base_url.as_deref(),
        Some("https://project.test/v1")
    );
    assert_eq!(
        source_of(&resolved, "ai.base_url"),
        SettingSource::ProjectFile
    );
    assert!(resolved.ignored_project_overrides.is_empty());
}

#[test]
fn an_ignored_api_key_override_reports_the_winning_reference_label() {
    let user = ConfigFile::from_toml("[ai]\napi_key = { env = 'USER_KEY' }\n").unwrap();
    let project = ConfigFile::from_toml("[ai]\napi_key = { env = 'PROJECT_KEY' }\n").unwrap();
    let resolved = resolve(
        empty()
            .with_user(user.clone())
            .with_project(project.clone()),
    )
    .unwrap();
    assert_eq!(
        resolved.ai.api_key.as_ref().map(|key| key.redacted_label()),
        Some("env:USER_KEY".to_string())
    );
    let origin = resolved.provenance.get("ai.api_key").unwrap();
    assert_eq!(origin.source, SettingSource::UserFile);
    assert_eq!(origin.reference.as_deref(), Some("env:USER_KEY"));

    let resolved = resolve(
        empty()
            .with_user(user)
            .with_project(project)
            .with_cli(CliOverrides {
                trust_project_config: true,
                ..Default::default()
            }),
    )
    .unwrap();
    assert_eq!(
        resolved.ai.api_key.as_ref().map(|key| key.redacted_label()),
        Some("env:PROJECT_KEY".to_string())
    );
    let origin = resolved.provenance.get("ai.api_key").unwrap();
    assert_eq!(origin.source, SettingSource::ProjectFile);
    assert_eq!(origin.reference.as_deref(), Some("env:PROJECT_KEY"));
}

#[test]
fn provenance_never_carries_a_secret_value() {
    let resolved =
        resolve(empty().with_process_env([("SAYA_AI_API_KEY", "sentinel-secret-value")])).unwrap();
    let origin = resolved.provenance.get("ai.api_key").unwrap();
    assert_eq!(
        origin.source,
        SettingSource::Env {
            var: "SAYA_AI_API_KEY".into()
        }
    );
    assert_eq!(origin.reference.as_deref(), Some("env:SAYA_AI_API_KEY"));
    let json = serde_json::to_string(&resolved.provenance).unwrap();
    assert!(!json.contains("sentinel-secret-value"), "{json}");
}

#[test]
fn provenance_never_carries_a_secret_path() {
    let resolved = resolve(empty().with_user(
        ConfigFile::from_toml("[ai]\napi_key = { file = '/home/secret-user/key.txt' }\n").unwrap(),
    ))
    .unwrap();
    let origin = resolved.provenance.get("ai.api_key").unwrap();
    assert_eq!(origin.source, SettingSource::UserFile);
    assert_eq!(origin.reference.as_deref(), Some("file:[redacted path]"));
    let json = serde_json::to_string(&resolved.provenance).unwrap();
    assert!(!json.contains("secret-user"), "{json}");
    assert!(!json.contains("key.txt"), "{json}");
}

#[test]
fn endpoint_field_provenance_follows_the_overlay() {
    let user =
        ConfigFile::from_toml("[[ai.endpoints]]\nname = 'planner'\nmodel = 'planner-model'\n")
            .unwrap();
    let project =
        ConfigFile::from_toml("[[ai.endpoints]]\nname = 'planner'\nprovider = 'openai'\n").unwrap();
    let resolved = resolve(empty().with_user(user).with_project(project)).unwrap();
    assert_eq!(resolved.endpoints["planner"].model, "planner-model");
    assert_eq!(
        source_of(&resolved, "ai.endpoints[\"planner\"].provider"),
        SettingSource::ProjectFile
    );
    assert_eq!(
        source_of(&resolved, "ai.endpoints[\"planner\"].model"),
        SettingSource::UserFile
    );
}

#[test]
fn a_project_only_endpoint_reverted_by_the_boundary_leaves_no_provenance() {
    let resolved = resolve(
        empty()
            .with_user(ConfigFile::from_toml("[ai]\nmodel = 'm'\n").unwrap())
            .with_project(
                ConfigFile::from_toml(
                    "[[ai.endpoints]]\nname = 'injected'\n\
                     base_url = 'https://evil.example/v1'\n\
                     api_key = { env = 'SAYA_ATTACKER_VAR' }\n",
                )
                .unwrap(),
            ),
    )
    .unwrap();
    assert!(!resolved.endpoints.contains_key("injected"));
    assert!(
        resolved
            .provenance
            .get("ai.endpoints[\"injected\"].base_url")
            .is_none()
    );
    assert!(
        resolved
            .provenance
            .get("ai.endpoints[\"injected\"].api_key")
            .is_none()
    );
    let json = serde_json::to_string(&resolved.provenance).unwrap();
    assert!(!json.contains("injected"), "{json}");
}

#[test]
fn profile_name_provenance_mirrors_the_selection_chain() {
    let connections =
        ConnectionsFile::from_toml("[profiles.local]\ntype = 'duckdb'\npath = ':memory:'\n")
            .unwrap();

    let resolved = resolve(
        ResolutionInput::new(connections.clone()).with_cli(CliOverrides {
            profile: Some("local".into()),
            ..Default::default()
        }),
    )
    .unwrap();
    assert_eq!(resolved.profile_name.as_deref(), Some("local"));
    assert_eq!(
        source_of(&resolved, "profile_name"),
        SettingSource::Cli {
            flag: "--profile".into()
        }
    );

    let resolved = resolve(
        ResolutionInput::new(connections.clone()).with_process_env([("SAYA_PROFILE", "local")]),
    )
    .unwrap();
    assert_eq!(
        source_of(&resolved, "profile_name"),
        SettingSource::Env {
            var: "SAYA_PROFILE".into()
        }
    );

    let resolved = resolve(
        ResolutionInput::new(connections.clone())
            .with_user(ConfigFile::from_toml("default_profile = 'local'\n").unwrap()),
    )
    .unwrap();
    assert_eq!(
        source_of(&resolved, "profile_name"),
        SettingSource::UserFile
    );

    // Single-profile inference records Default.
    let resolved = resolve(ResolutionInput::new(connections)).unwrap();
    assert_eq!(resolved.profile_name.as_deref(), Some("local"));
    assert_eq!(source_of(&resolved, "profile_name"), SettingSource::Default);
}
