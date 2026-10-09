use super::*;

/// `[jobs.runner]` (M5-4) resolves to no programs and the conservative
/// timeout when absent, and the declared keys resolve when present. The
/// empty allow is the point: there is no default program universe a run
/// gets for free — approving programs is a deliberate act.
#[test]
fn jobs_runner_resolves_defaults_and_declared_values() {
    let resolved =
        resolve(ResolutionInput::new(ConnectionsFile::default())).expect("resolution succeeds");
    assert_eq!(
        resolved.jobs.runner,
        saya_config::ResolvedRunnerJobs::default(),
        "the absent [jobs.runner] resolves to the conservative defaults"
    );
    assert!(
        resolved.jobs.runner.allow.is_empty(),
        "no default runner programs: {:?}",
        resolved.jobs.runner.allow
    );

    let declared = resolve_with_user(&format!(
        "[jobs.runner]\nallow = [\"duckdb\", \"jq\"]\ntimeout_seconds = 60\nprogram_dir = {}\n",
        toml_string(runner_program_dir())
    ));
    assert_eq!(
        declared.jobs.runner.allow,
        vec!["duckdb".to_owned(), "jq".to_owned()]
    );
    assert_eq!(declared.jobs.runner.timeout_seconds, 60);
    assert_eq!(
        declared.jobs.runner.program_dir.as_deref(),
        Some(std::path::Path::new(runner_program_dir())),
        "the staged programs' directory resolves as declared"
    );
}

/// A `[jobs.runner]` value below the floor is a typed resolve error naming
/// the field — a zero-second timeout would kill every child before its first
/// byte, a typo, not an intent. Never a silent clamp.
#[test]
fn jobs_runner_timeout_below_one_is_rejected_naming_the_field() {
    let error = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml("[jobs.runner]\ntimeout_seconds = 0\n").unwrap()),
    )
    .unwrap_err();
    assert!(
        matches!(error, ConfigError::SettingBelowMinimum { min: 1, .. }),
        "expected SettingBelowMinimum for a zero runner timeout, got {error:?}"
    );
    let display = format!("{error}");
    assert!(
        display.contains("runner.timeout_seconds"),
        "error must name the field: {display}"
    );
}

/// A `[jobs.runner] allow` entry that is not a bare program name is refused
/// at resolve time with the reason — a path-shaped or control-carrying entry
/// can never name a program the runner will run.
#[test]
fn jobs_runner_allow_outside_the_name_shape_is_rejected() {
    for (toml, program) in [
        ("[jobs.runner]\nallow = [\"/bin/echo\"]\n", "/bin/echo"),
        ("[jobs.runner]\nallow = [\"two words\"]\n", "two words"),
        ("[jobs.runner]\nallow = [\"\"]\n", ""),
    ] {
        let error = resolve(
            ResolutionInput::new(ConnectionsFile::default())
                .with_user(ConfigFile::from_toml(toml).expect("fixture must parse")),
        )
        .unwrap_err();
        assert!(
            matches!(&error, ConfigError::InvalidRunnerProgram { program: p, .. } if p == program),
            "expected InvalidRunnerProgram for {program:?}, got {error:?}"
        );
    }
}

/// A shell or interpreter must not look approved: the runner refuses those
/// structurally (an interpreter spawns arbitrary children from inside the
/// allowlist), so the config refuses the name at resolve time with the
/// reason named.
#[test]
fn jobs_runner_allow_refuses_shells_and_interpreters() {
    for toml in [
        "[jobs.runner]\nallow = [\"bash\"]\n",
        "[jobs.runner]\nallow = [\"sh\"]\n",
        "[jobs.runner]\nallow = [\"python3\"]\n",
        "[jobs.runner]\nallow = [\"env\"]\n",
    ] {
        let error = resolve(
            ResolutionInput::new(ConnectionsFile::default())
                .with_user(ConfigFile::from_toml(toml).expect("fixture must parse")),
        )
        .unwrap_err();
        let display = format!("{error}");
        assert!(
            matches!(&error, ConfigError::InvalidRunnerProgram { .. }),
            "expected InvalidRunnerProgram for {toml:?}, got {error:?}"
        );
        assert!(
            display.contains("interpreter"),
            "the refusal must say why shells and interpreters are refused: {display}"
        );
    }
}

/// A repeated `[jobs.runner] allow` entry is a typo, not a wider approval:
/// rejected rather than silently de-duplicated.
#[test]
fn jobs_runner_allow_refuses_duplicates() {
    let error = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml("[jobs.runner]\nallow = [\"jq\", \"jq\"]\n").unwrap()),
    )
    .unwrap_err();
    assert!(
        matches!(&error, ConfigError::InvalidRunnerProgram { program, .. } if program == "jq"),
        "expected InvalidRunnerProgram for the duplicate, got {error:?}"
    );
}

/// The `[jobs.runner] allow` list is bounded like every set-valued approval
/// surface the run contracts carry (`MAX_RUNNER_PROGRAMS`).
#[test]
fn jobs_runner_allow_above_the_contract_count_is_rejected() {
    let entries = (0..33)
        .map(|i| format!("\"prog-{i}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let error = resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(
        ConfigFile::from_toml(&format!("[jobs.runner]\nallow = [{entries}]\n")).unwrap(),
    ))
    .unwrap_err();
    assert!(
        matches!(
            error,
            ConfigError::SettingAboveMaximum {
                field: "runner.allow",
                value: 33,
                max: 32
            }
        ),
        "expected SettingAboveMaximum for 33 programs, got {error:?}"
    );
}

/// An unknown key inside `[jobs.runner]` is rejected at parse time with the
/// offending key named — `deny_unknown_fields` reaches the sub-table too.
#[test]
fn jobs_runner_unknown_keys_are_rejected_naming_the_key() {
    let error = ConfigFile::from_toml("[jobs.runner]\nfrobnicate = 1\n")
        .expect_err("an unknown [jobs.runner] key must be rejected");
    let display = format!("{error}");
    assert!(
        display.contains("frobnicate"),
        "the parse error must name the unknown key: {display}"
    );
}
