use super::*;

/// The interpreter family's admission check, name-level: a program the
/// run's interpreter scope carries but resolved `[jobs.interpreter] allow`
/// never declared refuses — `--allow` may only draw from the trusted
/// config's universe.
#[test]
fn interpreter_admission_refuses_a_name_outside_the_universe() {
    let programs = temp_dir("interpreter-admission-universe");
    let error = admit_interpreters(
        &interpreter_scope(&["python3"]),
        &["bash".to_owned()],
        &programs,
        Duration::from_secs(300),
    )
    .expect_err("a name the universe never declared must refuse");
    assert!(
        error.contains("python3") && error.contains(programs.display().to_string().as_str()),
        "the refusal must name the program and the directory: {error}"
    );
    assert!(
        error.contains("[jobs.interpreter] allow"),
        "the refusal must point at the universe that decides: {error}"
    );
    let _ = fs::remove_dir_all(&programs);
}

/// The interpreter family's admission check, file-level: an unstaged name
/// refuses with the staging instruction, and a staged regular file admits —
/// the same battery `validate_interpreter_call` applies per call, applied
/// once, at start.
#[cfg(unix)]
#[test]
fn interpreter_admission_refuses_unstaged_and_admits_a_staged_binary() {
    let programs = temp_dir("interpreter-admission-files");
    let error = admit_interpreters(
        &interpreter_scope(&["python3"]),
        &["python3".to_owned()],
        &programs,
        Duration::from_secs(300),
    )
    .expect_err("an unstaged interpreter refuses the run at start");
    assert!(
        error.contains("python3") && error.contains("not staged in"),
        "the refusal must name the program and the staging contract: {error}"
    );

    fs::copy("/bin/echo", programs.join("python3")).unwrap();
    admit_interpreters(
        &interpreter_scope(&["python3"]),
        &["python3".to_owned()],
        &programs,
        Duration::from_secs(300),
    )
    .expect("a staged regular binary admits");
    let _ = fs::remove_dir_all(&programs);
}
