//! `[instructions]` — which instruction sources load and their byte limits.
//!
//! Defaults follow the frozen instruction decisions: project instructions
//! (workspace-root AGENTS.md) and personal instructions both load, capped at
//! 32 KiB / 16 KiB with a 48 KiB aggregate. Each limit may be lowered but
//! never raised above its default — the defaults are the ceilings. The
//! personal file location is never a config value: it is derived from the
//! user config directory, so no config layer (especially the untrusted
//! project layer) can redirect it. Enablement and limits are protected
//! settings: a project layer's declaration is reverted and reported unless
//! `--trust-project-config`.

use std::path::Path;

use saya_config::{
    CliOverrides, ConfigError, ConfigFile, ConnectionsFile, DEFAULT_PERSONAL_INSTRUCTION_BYTES,
    DEFAULT_PROJECT_INSTRUCTION_BYTES, DEFAULT_TOTAL_INSTRUCTION_BYTES, MIN_INSTRUCTION_BYTES,
    ResolutionInput, personal_instructions_path, resolve,
};

fn resolve_with_user(toml: &str) -> saya_config::ResolvedConfig {
    resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml(toml).expect("fixture must parse")),
    )
    .expect("resolution succeeds")
}

/// Absent from the config, both sources load with the default ceilings —
/// a user who sets nothing gets the frozen-decision defaults.
#[test]
fn instructions_default_to_both_enabled_with_the_default_limits() {
    let resolved =
        resolve(ResolutionInput::new(ConnectionsFile::default())).expect("resolution succeeds");
    assert!(resolved.instructions.project_enabled);
    assert!(resolved.instructions.personal_enabled);
    assert_eq!(resolved.instructions.project_max_bytes, 32 * 1024);
    assert_eq!(resolved.instructions.personal_max_bytes, 16 * 1024);
    assert_eq!(resolved.instructions.total_max_bytes, 48 * 1024);
}

/// The user layer owns enablement and limits: it may disable either source
/// and lower any limit below its default.
#[test]
fn user_layer_disables_sources_and_lowers_limits() {
    let resolved = resolve_with_user(
        "[instructions]\nproject = false\npersonal = false\n\
         project_max_bytes = 4096\npersonal_max_bytes = 2048\ntotal_max_bytes = 8192\n",
    );
    assert!(!resolved.instructions.project_enabled);
    assert!(!resolved.instructions.personal_enabled);
    assert_eq!(resolved.instructions.project_max_bytes, 4096);
    assert_eq!(resolved.instructions.personal_max_bytes, 2048);
    assert_eq!(resolved.instructions.total_max_bytes, 8192);
}

/// Each limit may be lowered but never raised above its default — the
/// default is the ceiling. A raised limit is rejected at resolve time with
/// a typed error naming the field and the accepted range, for every limit.
#[test]
fn a_limit_above_its_default_is_rejected_naming_the_field_and_range() {
    for (field, max) in [
        ("project_max_bytes", DEFAULT_PROJECT_INSTRUCTION_BYTES),
        ("personal_max_bytes", DEFAULT_PERSONAL_INSTRUCTION_BYTES),
        ("total_max_bytes", DEFAULT_TOTAL_INSTRUCTION_BYTES),
    ] {
        let toml = format!("[instructions]\n{field} = {}\n", max + 1);
        let error = resolve(
            ResolutionInput::new(ConnectionsFile::default())
                .with_user(ConfigFile::from_toml(&toml).unwrap()),
        )
        .unwrap_err();
        let expected_field = format!("instructions.{field}");
        assert!(
            matches!(error, ConfigError::SettingOutOfRange { .. }),
            "expected SettingOutOfRange for {field} = {}: {error:?}",
            max + 1
        );
        assert!(
            matches!(
                &error,
                ConfigError::SettingOutOfRange { field, value, min, max }
                    if *field == expected_field
                        && *value == max + 1
                        && *min == MIN_INSTRUCTION_BYTES
            ),
            "the error must name the field, the value, and the floor: {error:?}"
        );
        let display = format!("{error}");
        assert!(
            display.contains(&format!("{}..={}", MIN_INSTRUCTION_BYTES, max)),
            "error must name the accepted range {min}..={max}: {display}",
            min = MIN_INSTRUCTION_BYTES,
            max = max
        );
    }
}

/// A limit below `MIN_INSTRUCTION_BYTES` is likewise a typed resolve error:
/// too small to hold a meaningful instruction file, a typo, not an intent.
#[test]
fn a_limit_below_the_minimum_is_rejected_naming_the_field_and_range() {
    let error = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml("[instructions]\nproject_max_bytes = 255\n").unwrap()),
    )
    .unwrap_err();
    assert!(
        matches!(
            error,
            ConfigError::SettingOutOfRange {
                field: "instructions.project_max_bytes",
                value: 255,
                min: 256,
                ..
            }
        ),
        "expected SettingOutOfRange for project_max_bytes = 255, got {error:?}"
    );
    assert!(
        matches!(
            &error,
            ConfigError::SettingOutOfRange { max, .. } if *max == DEFAULT_PROJECT_INSTRUCTION_BYTES
        ),
        "the ceiling must be the project default, got {error:?}"
    );
}

/// The bounds are inclusive: exactly the default and exactly the minimum
/// resolve, matching the crate's range-check style.
#[test]
fn instruction_limits_are_inclusive_bounds() {
    let toml = format!(
        "[instructions]\nproject_max_bytes = {}\npersonal_max_bytes = {}\n\
         total_max_bytes = {}\n",
        DEFAULT_PROJECT_INSTRUCTION_BYTES,
        DEFAULT_PERSONAL_INSTRUCTION_BYTES,
        DEFAULT_TOTAL_INSTRUCTION_BYTES
    );
    let resolved = resolve_with_user(&toml);
    assert_eq!(
        resolved.instructions.project_max_bytes,
        DEFAULT_PROJECT_INSTRUCTION_BYTES
    );
    let floor = format!("[instructions]\npersonal_max_bytes = {MIN_INSTRUCTION_BYTES}\n");
    let resolved = resolve_with_user(&floor);
    assert_eq!(
        resolved.instructions.personal_max_bytes,
        MIN_INSTRUCTION_BYTES
    );
}

/// The project layer cannot change enablement or limits: a repo's
/// `.saya/config.toml` is untrusted input, and these settings decide what
/// instruction context the model is assembled from. A project declaration is
/// reverted and reported by its dotted name.
#[test]
fn project_layer_cannot_change_enablement_or_limits() {
    let input = ResolutionInput::new(ConnectionsFile::default()).with_project(
        ConfigFile::from_toml("[instructions]\nproject = false\npersonal = false\n").unwrap(),
    );
    let resolved = resolve(input).expect("resolution succeeds");
    assert!(
        resolved.instructions.project_enabled,
        "project stays enabled"
    );
    assert!(
        resolved.instructions.personal_enabled,
        "personal stays enabled"
    );
    assert!(
        resolved
            .ignored_project_overrides
            .contains(&"instructions.project".to_string())
            && resolved
                .ignored_project_overrides
                .contains(&"instructions.personal".to_string()),
        "the ignored overrides must be reported: {:?}",
        resolved.ignored_project_overrides
    );
}

/// An out-of-range project-layer limit never becomes a resolve error for the
/// user: the revert happens before validation, so the untrusted value is
/// dropped, the default stands, and the override is reported.
#[test]
fn an_out_of_range_project_limit_is_reverted_before_validation() {
    let input = ResolutionInput::new(ConnectionsFile::default()).with_project(
        ConfigFile::from_toml("[instructions]\ntotal_max_bytes = 9999999\n").unwrap(),
    );
    let resolved = resolve(input).expect("resolution succeeds");
    assert_eq!(
        resolved.instructions.total_max_bytes,
        DEFAULT_TOTAL_INSTRUCTION_BYTES
    );
    assert!(
        resolved
            .ignored_project_overrides
            .contains(&"instructions.total_max_bytes".to_string()),
        "the ignored override must be reported: {:?}",
        resolved.ignored_project_overrides
    );
}

/// With `--trust-project-config`, the project layer's declaration is
/// honoured — the protected list is a default about who speaks first, not a
/// judgment that project config is useless.
#[test]
fn explicit_trust_honours_the_project_declaration() {
    let input = ResolutionInput::new(ConnectionsFile::default())
        .with_project(
            ConfigFile::from_toml("[instructions]\nproject = false\npersonal_max_bytes = 4096\n")
                .unwrap(),
        )
        .with_cli(CliOverrides {
            trust_project_config: true,
            ..Default::default()
        });
    let resolved = resolve(input).expect("resolution succeeds");
    assert!(!resolved.instructions.project_enabled);
    assert_eq!(resolved.instructions.personal_max_bytes, 4096);
    assert!(resolved.ignored_project_overrides.is_empty());
}

/// The personal file location is never a config value: a path-like key under
/// `[instructions]` is refused at parse (the crate's unknown-key policy), and
/// the derived path stays exactly `<user_config_dir>/instructions.md` — no
/// config layer can redirect it.
#[test]
fn a_path_like_key_is_refused_and_the_personal_path_is_unchanged() {
    let error =
        ConfigFile::from_toml("[instructions]\npersonal_path = '/etc/passwd'\n").unwrap_err();
    assert!(
        error.to_string().contains("personal_path"),
        "the unknown key must be named: {error}"
    );
    assert_eq!(
        personal_instructions_path(Path::new("/cfg/saya")),
        std::path::PathBuf::from("/cfg/saya/instructions.md"),
        "the derived path must stay inside the user config directory"
    );
}
