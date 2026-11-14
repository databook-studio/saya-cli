//! `[permissions] allow`/`deny` resolution: trusted rules parse, invalid
//! entries are diagnostics, and flag rules add to config rules.

use saya_config::{
    CliOverrides, ConfigFile, ConnectionsFile, PermissionsFile, ResolutionInput, resolve,
};

fn with_user(toml: &str) -> ResolutionInput {
    ResolutionInput::new(ConnectionsFile::default())
        .with_user(ConfigFile::from_toml(toml).expect("the fixture must parse"))
}

#[test]
fn trusted_lists_resolve_to_rules() {
    let resolved = resolve(with_user(
        "[permissions]\nallow = ['cargo test', 'git diff']\ndeny = ['rm']\n",
    ))
    .expect("resolution succeeds");
    let rules = &resolved.permissions.rules;
    assert_eq!(
        rules
            .allow
            .iter()
            .map(|rule| rule.text())
            .collect::<Vec<_>>(),
        ["cargo test", "git diff"]
    );
    assert_eq!(
        rules
            .deny
            .iter()
            .map(|rule| rule.text())
            .collect::<Vec<_>>(),
        ["rm"]
    );
    assert!(rules.diagnostics.is_empty());
}

#[test]
fn invalid_entries_are_diagnostics_not_errors() {
    let long = "x".repeat(saya_config::MAX_COMMAND_RULE_BYTES + 1);
    let toml = format!("[permissions]\nallow = ['', \"cargo\\ttest\", '{long}', 'cargo test']\n");
    let resolved = resolve(with_user(&toml)).expect("a bad rule is not a resolve error");
    let rules = &resolved.permissions.rules;
    assert_eq!(
        rules
            .allow
            .iter()
            .map(|rule| rule.text())
            .collect::<Vec<_>>(),
        ["cargo test"],
        "the valid rule survives"
    );
    assert_eq!(rules.diagnostics.len(), 3, "{:?}", rules.diagnostics);
    for diagnostic in &rules.diagnostics {
        assert!(
            diagnostic.starts_with("permissions.allow["),
            "the diagnostic names the entry: {diagnostic}"
        );
    }
}

#[test]
fn the_rule_count_is_bounded_with_a_diagnostic() {
    let entries = (0..=saya_config::MAX_COMMAND_RULES)
        .map(|index| format!("'cmd {index}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let resolved = resolve(with_user(&format!("[permissions]\nallow = [{entries}]\n")))
        .expect("resolution succeeds");
    let rules = &resolved.permissions.rules;
    assert_eq!(rules.allow.len(), saya_config::MAX_COMMAND_RULES);
    assert!(
        rules
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("more than")),
        "{:?}",
        rules.diagnostics
    );
}

#[test]
fn flag_rules_add_to_config_rules() {
    let input = with_user("[permissions]\nallow = ['cargo test']\n").with_cli(CliOverrides {
        permissions: PermissionsFile {
            allow: Some(vec!["git diff".into()]),
            deny: Some(vec!["rm".into()]),
            ..Default::default()
        },
        ..Default::default()
    });
    let resolved = resolve(input).expect("resolution succeeds");
    let rules = &resolved.permissions.rules;
    assert_eq!(
        rules
            .allow
            .iter()
            .map(|rule| rule.text())
            .collect::<Vec<_>>(),
        ["cargo test", "git diff"],
        "the flag adds after the config rule"
    );
    assert_eq!(
        rules
            .deny
            .iter()
            .map(|rule| rule.text())
            .collect::<Vec<_>>(),
        ["rm"]
    );
}
