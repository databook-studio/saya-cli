//! `[ai] reasoning_effort` — the effort setting.
//!
//! Unset and `adaptive` both mean the loop's per-turn policy (high on the first
//! request and after a failed tool result, low otherwise). The resolved value
//! stays an `Option`: `None` is the adaptive policy, and a stated value pins
//! every request.

use saya_config::{ConfigFile, ConnectionsFile, ReasoningEffort, ResolutionInput, resolve};

fn resolve_user(toml: &str) -> saya_config::ResolvedConfig {
    let config = ConfigFile::from_toml(toml).expect("the config parses");
    resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(config))
        .expect("the config resolves")
}

/// Each spelling resolves to its own value. Asserted through the serialized
/// diagnostics as well as the resolved field, because the diagnostics view is
/// what `config show` prints and it must not be able to disagree with the
/// setting the provider is built from.
#[test]
fn each_reasoning_effort_spelling_resolves_to_its_value() {
    for (spelling, expected) in [
        ("adaptive", ReasoningEffort::Adaptive),
        ("low", ReasoningEffort::Low),
        ("medium", ReasoningEffort::Medium),
        ("high", ReasoningEffort::High),
    ] {
        let resolved = resolve_user(&format!("[ai]\nreasoning_effort = '{spelling}'\n"));
        assert_eq!(resolved.ai.reasoning_effort, Some(expected), "{spelling}");
        let rendered = serde_json::to_value(resolved.redacted_diagnostics()).unwrap();
        assert_eq!(rendered["reasoning_effort"], spelling);
    }
}

/// Unset is the default and is not a value: no layer stated an effort, so the
/// resolved setting is `None` and the diagnostics say so.
#[test]
fn an_unset_reasoning_effort_resolves_to_none() {
    let resolved = resolve_user("[ai]\nmodel = 'm'\n");
    assert_eq!(resolved.ai.reasoning_effort, None);
    let rendered = serde_json::to_value(resolved.redacted_diagnostics()).unwrap();
    assert!(rendered["reasoning_effort"].is_null());
}

/// A spelling outside the accepted set is a typed parse error that lists the
/// ones that are accepted — the user gets the fix, not a silent fallback to
/// "send nothing" that would look like the setting worked. The rejected value
/// itself is not echoed, like every other parse message this crate renders:
/// the line and column say where to look.
#[test]
fn an_unknown_reasoning_effort_is_rejected_and_lists_the_accepted_values() {
    let error = ConfigFile::from_toml("[ai]\nreasoning_effort = 'extreme'\n")
        .expect_err("an unknown spelling must fail");
    let message = error.to_string();
    for accepted in ["adaptive", "low", "medium", "high"] {
        assert!(
            message.contains(accepted),
            "the error must list {accepted}: {message}"
        );
    }
    assert!(
        message.contains("line 2"),
        "the error must locate the offending line: {message}"
    );
}

/// `minimal` and `default` are refused rather than accepted-and-ignored.
/// `minimal` is the OpenAI-family spelling some gateways reject with HTTP 400,
/// so offering it as a setting would offer a value that can fail every request;
/// `default` would be a second name for the unset state, and `adaptive` already
/// names that policy.
#[test]
fn minimal_and_default_are_not_settable_spellings() {
    for spelling in ["minimal", "default"] {
        let error = ConfigFile::from_toml(&format!("[ai]\nreasoning_effort = '{spelling}'\n"))
            .expect_err("neither spelling may be settable");
        let message = error.to_string();
        assert!(
            message.contains("expected one of `adaptive`, `low`, `medium`, `high`"),
            "{spelling} must be refused against the accepted set: {message}"
        );
    }
}

/// The setting follows the ordinary `[ai]` layering: project over user.
#[test]
fn reasoning_effort_follows_the_existing_config_layering() {
    let user = ConfigFile::from_toml("[ai]\nreasoning_effort = 'low'\n").unwrap();
    let project = ConfigFile::from_toml("[ai]\nreasoning_effort = 'high'\n").unwrap();
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(user)
            .with_project(project),
    )
    .unwrap();
    assert_eq!(resolved.ai.reasoning_effort, Some(ReasoningEffort::High));
}

/// Provenance is recorded like every neighbouring key: the layer that stated
/// it, or `Default` when nobody did.
#[test]
fn reasoning_effort_records_its_provenance() {
    let stated = resolve_user("[ai]\nreasoning_effort = 'medium'\n");
    let origin = stated
        .provenance
        .get("ai.reasoning_effort")
        .expect("the key is recorded");
    assert_eq!(origin.source, saya_config::SettingSource::UserFile);

    let unstated = resolve_user("[ai]\nmodel = 'm'\n");
    let origin = unstated
        .provenance
        .get("ai.reasoning_effort")
        .expect("the key is recorded");
    assert_eq!(origin.source, saya_config::SettingSource::Default);
}
