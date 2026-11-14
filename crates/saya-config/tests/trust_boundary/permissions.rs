use super::*;

/// A repository cannot make `bypass` reachable. `[permissions] allow_bypass`
/// is the one key that decides whether the session's `bypass` mode can be
/// entered at all, and a model-writable `.saya/config.toml` stating it is
/// reverted and reported.
#[test]
fn project_layer_cannot_allow_bypass() {
    let input = project_with("[permissions]\nallow_bypass = true\n");
    let resolved = resolve(input).expect("resolution succeeds");
    assert!(
        !resolved.permissions.allow_bypass,
        "the project layer's consent is ignored"
    );
    assert!(
        resolved
            .ignored_project_overrides
            .iter()
            .any(|name| name == "permissions"),
        "the ignored override must be reported: {:?}",
        resolved.ignored_project_overrides
    );
}

/// A repository cannot state command rules either: they pre-answer the very
/// questions the per-call consent exists for, and `deny` could herd the
/// session's work onto the unsandboxed host lane.
#[test]
fn project_layer_cannot_state_command_rules() {
    let input = project_with("[permissions]\nallow = ['cargo test']\ndeny = ['git diff']\n");
    let resolved = resolve(input).expect("resolution succeeds");
    assert!(
        resolved.permissions.rules.allow.is_empty() && resolved.permissions.rules.deny.is_empty(),
        "the project layer's rules are ignored: {:?}",
        resolved.permissions.rules
    );
    assert!(
        resolved
            .ignored_project_overrides
            .iter()
            .any(|name| name == "permissions"),
        "the ignored override must be reported: {:?}",
        resolved.ignored_project_overrides
    );
}

/// The user layer may state it: the boundary is about *who* may, not about
/// whether the setting exists. Absent is `false` — the default, and the only
/// value every surface had before the key existed.
#[test]
fn user_layer_may_allow_bypass_and_absent_is_false() {
    let input = ResolutionInput::new(ConnectionsFile::default())
        .with_user(ConfigFile::from_toml("[permissions]\nallow_bypass = true\n").unwrap());
    let resolved = resolve(input).expect("resolution succeeds");
    assert!(resolved.permissions.allow_bypass);
    assert!(resolved.ignored_project_overrides.is_empty());

    let absent = resolve(ResolutionInput::new(ConnectionsFile::default()))
        .expect("a config that states nothing resolves");
    assert!(
        !absent.permissions.allow_bypass,
        "the default is the closed direction"
    );
}

/// The winning layer is attributed like every other user-facing setting, and
/// the project layer's attempt is rolled back to it.
#[test]
fn allow_bypass_reports_its_layer_and_survives_a_project_reversion() {
    let input = ResolutionInput::new(ConnectionsFile::default())
        .with_user(ConfigFile::from_toml("[permissions]\nallow_bypass = true\n").unwrap())
        .with_project(ConfigFile::from_toml("[permissions]\nallow_bypass = false\n").unwrap());
    let resolved = resolve(input).expect("resolution succeeds");
    assert!(
        resolved.permissions.allow_bypass,
        "the user's value survives the project layer's attempt to withdraw it"
    );
    assert_eq!(
        resolved
            .provenance
            .get("permissions.allow_bypass")
            .map(|origin| &origin.source),
        Some(&saya_config::SettingSource::UserFile),
        "the key names the layer that supplied it"
    );

    let default = resolve(ResolutionInput::new(ConnectionsFile::default())).unwrap();
    assert_eq!(
        default
            .provenance
            .get("permissions.allow_bypass")
            .map(|origin| &origin.source),
        Some(&saya_config::SettingSource::Default),
        "an unstated key still reports an origin"
    );
}
