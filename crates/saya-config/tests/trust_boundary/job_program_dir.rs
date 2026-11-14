use super::*;

/// `[job] program_dir` names the trusted directory the job host and its
/// extensions launch programs from: the bytes that answer an approved
/// program's name. It carries the wholesale protection `[coding]` had, so a
/// repository-controlled config cannot point the host at a directory of its
/// own choosing — the revert names the section it rode in on, `job`.
#[test]
fn project_layer_cannot_set_the_job_program_dir() {
    let input = project_with("[job]\nprogram_dir = '/tmp/attacker-programs'\n");
    let resolved = resolve(input).expect("resolution succeeds");
    assert_eq!(
        resolved.coding.program_dir, None,
        "the untrusted directory must be reverted"
    );
    assert!(
        resolved
            .ignored_project_overrides
            .contains(&"job".to_string()),
        "the report must name the protected section: {:?}",
        resolved.ignored_project_overrides
    );
}

/// The same key from the trusted layers resolves, and explicit trust
/// (`--trust-project-config`) restores the project's declaration — the
/// protected list is a default about who speaks first, not a judgment that
/// project config is useless.
#[test]
fn trusted_layers_may_set_the_job_program_dir() {
    let directory = if cfg!(windows) {
        "C:/saya-programs"
    } else {
        "/opt/saya-programs"
    };
    let user = ConfigFile::from_toml(&format!("[job]\nprogram_dir = '{directory}'\n")).unwrap();
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(user)
            .with_project(ConfigFile::from_toml("[run]\nmax_rows = 7\n").unwrap()),
    )
    .expect("resolution succeeds");
    assert_eq!(
        resolved.coding.program_dir.as_deref(),
        Some(std::path::Path::new(directory)),
        "the trusted layer's directory resolves"
    );
    assert!(resolved.ignored_project_overrides.is_empty());

    let input =
        project_with(&format!("[job]\nprogram_dir = '{directory}'\n")).with_cli(CliOverrides {
            trust_project_config: true,
            ..Default::default()
        });
    let resolved = resolve(input).expect("resolution succeeds");
    assert_eq!(
        resolved.coding.program_dir.as_deref(),
        Some(std::path::Path::new(directory)),
        "explicit trust honours the project's declaration"
    );
    assert!(resolved.ignored_project_overrides.is_empty());
}
