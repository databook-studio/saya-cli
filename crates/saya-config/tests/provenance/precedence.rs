//! The winning layer across the full precedence chain — user file, project
//! file, env file, process environment, CLI — and the built-in default when
//! nothing supplies the setting.

use saya_config::{CliOverrides, ConfigFile, SettingSource, resolve};

use super::{empty, source_of};

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
