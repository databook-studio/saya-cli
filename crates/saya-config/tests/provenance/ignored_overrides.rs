//! Untrusted project overrides keep the trusted layer's value and its
//! provenance; with the project layer trusted, the project wins and is
//! attributed to it — including the reference label an api_key reports.

use saya_config::{CliOverrides, ConfigFile, SettingSource, resolve};

use super::{empty, source_of};

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
