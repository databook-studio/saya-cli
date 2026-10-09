use super::*;

#[tokio::test]
async fn a_non_allowlisted_program_is_a_typed_error_and_nothing_is_spawned() {
    let outcome = standard()
        .tool()
        .run("printf", &["hello".to_owned()], None)
        .await;
    assert!(
        matches!(&outcome, Err(RunnerError::ProgramNotAllowlisted { program }) if program == "printf"),
        "the refusal must be the typed not-allowlisted variant: {outcome:?}"
    );
}

/// `bash` is refused even when a hand-built scope allowlists it: an
/// interpreter can spawn arbitrary children with arbitrary argv and
/// would void the typed-argv contract from inside the allowlist.
#[test]
fn bash_is_refused_even_when_allowlisted() {
    let scope = RunnerScope::new(vec!["bash".to_owned(), PROGRAM.to_owned()]).expect("shaped");
    let error = validate_call(
        &scope,
        &standard().programs,
        DEFAULT_TIMEOUT,
        None,
        "bash",
        &["-c".to_owned(), "echo pwned".to_owned()],
    )
    .expect_err("bash must be refused");
    assert!(
        matches!(&error, RunnerError::ProgramRefused { program, .. } if program == "bash"),
        "the refusal must name the program: {error}"
    );
}

#[test]
fn sh_is_refused_even_when_allowlisted() {
    let scope = RunnerScope::new(vec!["sh".to_owned(), PROGRAM.to_owned()]).expect("shaped");
    let error = validate_call(
        &scope,
        &standard().programs,
        DEFAULT_TIMEOUT,
        None,
        "sh",
        &["-c".to_owned(), "echo pwned".to_owned()],
    )
    .expect_err("sh must be refused");
    assert!(
        matches!(&error, RunnerError::ProgramRefused { program, .. } if program == "sh"),
        "the refusal must name the program: {error}"
    );
}

/// The wrapper trick: a script that execs an interpreter dies on its
/// shebang — the interpreter it would exec was never allowlisted.
#[test]
fn a_wrapper_script_is_refused() {
    let programs = leak("wrapper-programs");
    let wrapper = programs.join("wrapper");
    fs::write(&wrapper, "#!/bin/sh\nexec /bin/echo \"$@\"\n").expect("the wrapper must plant");
    let scope = RunnerScope::new(vec!["wrapper".to_owned(), PROGRAM.to_owned()]).expect("shaped");
    let error = validate_call(&scope, &programs, DEFAULT_TIMEOUT, None, "wrapper", &[])
        .expect_err("a wrapper script must be refused");
    assert!(
        matches!(&error, RunnerError::ProgramRefused { program, reason } if program == "wrapper" && reason.contains("script")),
        "the refusal must say why the wrapper died: {error}"
    );
}

/// The absolute-path trick: a lookalike binary planted outside the
/// program directory, named by its absolute path. The allowlist names
/// programs, never paths — the call is refused before anything spawns.
#[test]
fn an_absolute_path_to_a_lookalike_is_refused() {
    let lookalike = leak("lookalike").join("echo-lookalike");
    fs::copy("/bin/echo", &lookalike).expect("the lookalike must stage");
    let error = validate_call(
        &RunnerScope::new(vec![PROGRAM.to_owned()]).expect("shaped"),
        &standard().programs,
        DEFAULT_TIMEOUT,
        None,
        &lookalike.display().to_string(),
        &[],
    )
    .expect_err("a path-shaped program name must be refused");
    assert!(
        matches!(&error, RunnerError::ProgramRefused { reason, .. } if reason.contains("bare name")),
        "the refusal must say a program is a bare name: {error}"
    );
}
