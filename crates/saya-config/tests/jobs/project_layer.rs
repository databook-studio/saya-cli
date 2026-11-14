use super::*;

/// `[limits.run]` holds the `[jobs]` ceilings, and `[jobs]` is not
/// protected: a run ceiling is a cost control, not a security-critical
/// setting, so the project layer may still set it and the merge carries it
/// across layers. Protection is per sub-table, so `[limits.session]` and
/// `[limits.ask]` stay protected while this one does not.
#[test]
fn project_layer_run_limits_apply_and_are_not_protected() {
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml("[limits.run]\nturns = 40\n").unwrap())
            .with_project(ConfigFile::from_toml("[limits.run]\nturns = 7\n").unwrap()),
    )
    .expect("resolution succeeds");
    assert_eq!(resolved.limits.run.turns.value(), Some(7));
    assert!(
        resolved.ignored_project_overrides.is_empty(),
        "limits.run is not security-critical, so nothing may be reported as ignored: {:?}",
        resolved.ignored_project_overrides
    );
}
