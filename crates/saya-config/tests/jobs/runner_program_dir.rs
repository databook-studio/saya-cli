use super::*;

/// A relative `[jobs.runner] program_dir` is a typed resolve error: the
/// canonical form must not depend on the working directory the config was
/// loaded from, and the run's probe verdict is only as real as the one
/// directory it proved.
#[test]
fn jobs_runner_program_dir_relative_is_rejected_naming_the_field() {
    let error =
        resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(
            ConfigFile::from_toml("[jobs.runner]\nprogram_dir = \"programs\"\n").unwrap(),
        ))
        .unwrap_err();
    assert!(
        matches!(&error, ConfigError::RelativeRunnerProgramDir { path } if path == "programs"),
        "expected RelativeRunnerProgramDir, got {error:?}"
    );
    let display = format!("{error}");
    assert!(
        display.contains("runner.program_dir") && display.contains("absolute"),
        "the refusal must name the field and the requirement: {display}"
    );
}

/// `allow` that names programs requires `program_dir`: the runner resolves
/// every allowlisted program inside one directory, so an allowlist without
/// its directory approves programs that cannot run — a typed resolve error,
/// the same class every other `[jobs]` mistake gets.
#[test]
fn jobs_runner_allow_without_program_dir_is_rejected() {
    let error = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml("[jobs.runner]\nallow = [\"bench\"]\n").unwrap()),
    )
    .unwrap_err();
    assert!(
        matches!(error, ConfigError::RunnerAllowWithoutProgramDir),
        "expected RunnerAllowWithoutProgramDir, got {error:?}"
    );
    let display = format!("{error}");
    assert!(
        display.contains("program_dir"),
        "the refusal must name the missing key: {display}"
    );
}

/// `program_dir` alone (empty `allow`) is harmless and allowed: it declares
/// where programs would be staged without approving any program.
#[test]
fn jobs_runner_program_dir_alone_resolves() {
    let resolved = resolve_with_user(&format!(
        "[jobs.runner]\nprogram_dir = {}\n",
        toml_string(runner_program_dir())
    ));
    assert!(resolved.jobs.runner.allow.is_empty());
    assert_eq!(
        resolved.jobs.runner.program_dir.as_deref(),
        Some(std::path::Path::new(runner_program_dir()))
    );
}

/// A dangling `program_dir` resolves fine: existence is deliberately not a
/// resolve-time question, so `saya ask` and `saya query` are unaffected by
/// any state of the key; a run that approved the runner fails closed at
/// assemble instead.
#[test]
fn jobs_runner_dangling_program_dir_resolves() {
    let dangling_dir = if cfg!(windows) {
        r"C:\saya-this-directory-does-not-exist"
    } else {
        "/nonexistent/saya-programs"
    };
    let resolved = resolve_with_user(&format!(
        "[jobs.runner]\nallow = [\"bench\"]\nprogram_dir = {}\n",
        toml_string(dangling_dir)
    ));
    assert_eq!(
        resolved.jobs.runner.program_dir.as_deref(),
        Some(std::path::Path::new(dangling_dir))
    );
}
