use super::*;

/// The universe disjointness holds by construction (the interpreter
/// approval's design §1): a `[jobs.interpreter] allow` member the runner
/// does not refuse is a typed resolve error pointing at `[jobs.runner]
/// allow` — a non-refused name never rides the interpreter family — while
/// the `[jobs.runner]` entry keeps its exact error, so the two universes
/// cannot drift into each other.
#[test]
fn jobs_interpreter_allow_refuses_programs_the_runner_can_run() {
    for toml in [
        "[jobs.interpreter]\nallow = [\"ripgrep\"]\n",
        "[jobs.interpreter]\nallow = [\"bench\"]\n",
    ] {
        let error = resolve(
            ResolutionInput::new(ConnectionsFile::default())
                .with_user(ConfigFile::from_toml(toml).expect("fixture must parse")),
        )
        .unwrap_err();
        let display = format!("{error}");
        assert!(
            matches!(&error, ConfigError::InvalidInterpreterProgram { .. }),
            "expected InvalidInterpreterProgram for {toml:?}, got {error:?}"
        );
        assert!(
            display.contains("[jobs.runner] allow"),
            "the refusal must point at the runner's family: {display}"
        );
    }
}

/// `[jobs.interpreter] allow` requires the one program directory the
/// interpreters are staged in: the resolve refuses the combination with the
/// same typed error class the runner's own allow-without-dir refusal uses.
#[test]
fn jobs_interpreter_allow_without_program_dir_refuses() {
    let error = resolve(
        ResolutionInput::new(ConnectionsFile::default()).with_user(
            ConfigFile::from_toml("[jobs.interpreter]\nallow = [\"python3\"]\n")
                .expect("fixture must parse"),
        ),
    )
    .unwrap_err();
    assert!(
        matches!(&error, ConfigError::InterpreterAllowWithoutProgramDir),
        "expected InterpreterAllowWithoutProgramDir, got {error:?}"
    );
}
