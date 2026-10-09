use super::*;

/// `[jobs]` is a cost control, not a security-critical setting, so the
/// project layer may set it and the merge must carry it across layers.
#[test]
fn project_layer_jobs_applies_and_is_not_protected() {
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml("[jobs]\nturns = 40\n").unwrap())
            .with_project(ConfigFile::from_toml("[jobs]\nturns = 7\n").unwrap()),
    )
    .expect("resolution succeeds");
    assert_eq!(resolved.jobs.turns, Some(7));
    assert!(
        resolved.ignored_project_overrides.is_empty(),
        "jobs is not security-critical, so nothing may be reported as ignored: {:?}",
        resolved.ignored_project_overrides
    );
}
