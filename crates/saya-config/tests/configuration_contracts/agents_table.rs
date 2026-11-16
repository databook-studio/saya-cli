//! The top-level `[agents]` table: the child bound, the model-request bound and
//! the helper type registry in one table, the two reserved names that keep a
//! setting from being read as a type, and the retired `[coding]` spellings that
//! name it instead.
//!
//! Everything here is asserted through the parse result, the refusal text, the
//! resolved diagnostics and the ignored-override report — the surfaces a user
//! sees — so the tests say what the table *does*, not how it is stored.

use saya_config::{ConfigError, ConfigFile, ConnectionsFile, ResolutionInput, resolve};
use serde_json::{Value, json};

fn resolved(toml: &str) -> Result<saya_config::ResolvedConfig, ConfigError> {
    resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml(toml).expect("the fixture parses")),
    )
}

fn diagnostics(resolved: &saya_config::ResolvedConfig) -> Value {
    serde_json::to_value(resolved.redacted_diagnostics()).expect("the view serialises")
}

/// The table holds both settings and the named types: `max_children` and
/// `max_concurrent_model_requests` are settings, every other key is a helper
/// type, and one table serves both.
#[test]
fn the_agents_table_carries_both_settings_and_the_named_types() {
    let resolved = resolved(
        "[agents]\nmax_children = 2\nmax_concurrent_model_requests = 3\n\n\
         [agents.reviewer]\nrole = 'analyst'\n",
    )
    .expect("the table resolves");
    let effective = diagnostics(&resolved);
    assert_eq!(effective["agents_max_children"], json!(2));
    assert_eq!(effective["agents_max_concurrent_model_requests"], json!(3));
    assert_eq!(
        effective["agents"]["reviewer"],
        json!({"role": "analyst", "allow_write": false, "model": null}),
        "a named entry is a helper type, resolved beside the settings"
    );
    assert_eq!(
        effective["agents"]["explore"],
        json!({"role": "analyst", "allow_write": false, "model": null}),
        "the built-ins are still there"
    );
}

/// An empty config resolves the settings and the two built-ins exactly as it
/// did before the table existed.
#[test]
fn the_defaults_are_the_ones_they_were() {
    let effective =
        diagnostics(&resolve(ResolutionInput::new(ConnectionsFile::default())).unwrap());
    assert_eq!(effective["agents_max_children"], json!(3));
    assert_eq!(effective["agents_max_concurrent_model_requests"], json!(4));
    assert_eq!(
        effective["agents"],
        json!({
            "explore": {"role": "analyst", "allow_write": false, "model": null},
            "general": {"role": "implementer", "allow_write": true, "model": null},
        })
    );
}

/// The two setting names are reserved: an entry may not take one, because the
/// key would then mean two things at once. The refusal says so by name.
#[test]
fn a_reserved_name_cannot_name_a_helper_type() {
    for name in ["max_children", "max_concurrent_model_requests"] {
        let error = ConfigFile::from_toml(&format!("[agents.{name}]\nrole = 'analyst'\n"))
            .expect_err("a reserved name is refused");
        assert!(
            error.to_string().contains(name) && error.to_string().contains("reserved"),
            "the refusal names {name} and why: {error}"
        );
    }
}

/// A key in `[agents]` that is not a setting is a helper type, and a helper type
/// is a table. The refusal points at the spelling that works.
#[test]
fn a_key_that_is_not_a_setting_is_a_helper_type_table() {
    let error = ConfigFile::from_toml("[agents]\nreviewer = 'analyst'\n")
        .expect_err("a scalar where a type belongs is refused");
    assert!(
        error.to_string().contains("[agents.reviewer]"),
        "the refusal names the table to write: {error}"
    );
}

/// A setting that is not a whole number is refused by name rather than read as
/// a type, so a typo cannot quietly become an entry.
#[test]
fn a_setting_that_is_not_a_whole_number_is_refused() {
    let error = ConfigFile::from_toml("[agents]\nmax_children = 'three'\n")
        .expect_err("a non-numeric setting is refused");
    assert!(
        error.to_string().contains("agents.max_children"),
        "the refusal names the setting: {error}"
    );
}

/// Every `[coding]` spelling is retired with a message that names the new one:
/// there is no alias, and a stale file is told what to write instead.
#[test]
fn each_retired_coding_spelling_names_its_replacement() {
    for (toml, expected) in [
        ("[coding]\nmax_children = 2\n", "agents.max_children"),
        (
            "[coding]\nmax_concurrent_model_requests = 3\n",
            "agents.max_concurrent_model_requests",
        ),
        ("[coding.agents.reviewer]\nrole = 'analyst'\n", "[agents]"),
        ("[coding]\nshell = false\n", "[tools] allow"),
    ] {
        let error = ConfigFile::from_toml(toml).expect_err("a retired key is refused");
        assert!(
            error.to_string().contains(expected),
            "{toml} must name {expected}: {error}"
        );
    }
}
