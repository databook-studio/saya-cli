use super::*;

/// The interpreter universe is a protected setting (the interpreter
/// approval's design §2): which bytes answer an approved
/// `--allow interpreter:<program>` is staging input, and staging input from
/// an untrusted file would turn the user's typed approval into approval of
/// bytes the user never saw. A project layer declaring `[jobs.interpreter]`
/// is reverted and reported by its dotted name — an interpreter the trusted
/// layers never declared is still an attacker-chosen program.
#[test]
fn project_layer_cannot_declare_the_interpreter_universe() {
    let input = project_with("[jobs.interpreter]\nallow = ['python3']\n");
    let resolved = resolve(input).expect("resolution succeeds");
    assert!(
        resolved.jobs.interpreter.allow.is_empty(),
        "the untrusted universe must resolve empty: {:?}",
        resolved.jobs.interpreter.allow
    );
    assert!(
        resolved
            .ignored_project_overrides
            .contains(&"jobs.interpreter".to_string()),
        "the report must name the sub-table by its dotted name: {:?}",
        resolved.ignored_project_overrides
    );
}

/// The same sub-table from the trusted layers resolves, and explicit trust
/// (`--trust-project-config`) restores the project's declaration — the
/// protected list is a default about who speaks first, not a judgment that
/// project config is useless. Declaring interpreters requires the one
/// program directory they are staged in, so both trusted fixtures carry it.
#[test]
fn trusted_layers_may_declare_the_interpreter_universe() {
    let program_dir = if cfg!(windows) {
        "C:/saya-programs"
    } else {
        "/opt/saya-programs"
    };
    let user = ConfigFile::from_toml(&format!(
        "[jobs.runner]\nprogram_dir = '{program_dir}'\n\
         [jobs.interpreter]\nallow = ['python3']\n"
    ))
    .unwrap();
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(user)
            .with_project(ConfigFile::from_toml("[run]\nmax_rows = 7\n").unwrap()),
    )
    .expect("resolution succeeds");
    assert_eq!(
        resolved.jobs.interpreter.allow,
        vec!["python3".to_string()],
        "the trusted layer's universe resolves"
    );
    assert!(resolved.ignored_project_overrides.is_empty());

    let input = project_with(&format!(
        "[jobs.runner]\nprogram_dir = '{program_dir}'\n\
         [jobs.interpreter]\nallow = ['python3']\n"
    ))
    .with_cli(CliOverrides {
        trust_project_config: true,
        ..Default::default()
    });
    let resolved = resolve(input).expect("resolution succeeds");
    assert_eq!(
        resolved.jobs.interpreter.allow,
        vec!["python3".to_string()],
        "explicit trust honours the project's declaration"
    );
    assert!(resolved.ignored_project_overrides.is_empty());
}
