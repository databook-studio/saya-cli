use super::*;

/// The allowlist is per step: a program inside the run's approval but
/// outside the step's narrowed scope is refused, while the step's own
/// program runs.
#[tokio::test]
async fn the_allowlist_is_the_step_scope_narrowed_from_the_run_approval() {
    let run_approved =
        RunnerScope::new(vec![PROGRAM.to_owned(), "second".to_owned()]).expect("shaped");
    let step_narrowed = RunnerScope::new(vec![PROGRAM.to_owned()]).expect("shaped");
    let error = standard()
        .tool_with_scope(step_narrowed)
        .run("second", &[], None)
        .await
        .expect_err("a run-approved program outside the step scope is refused");
    assert!(
        matches!(&error, RunnerError::ProgramNotAllowlisted { program } if program == "second"),
        "the refusal must be the typed not-allowlisted variant: {error}"
    );
    let outcome = standard()
        .tool_with_scope(run_approved)
        .run(PROGRAM, &["exit".to_owned(), "0".to_owned()], Some(30))
        .await
        .expect("the step's own program runs");
    assert_eq!(outcome.exit_code, Some(0));
}

#[tokio::test]
async fn cwd_is_pinned_to_the_run_workspace() {
    let outcome = standard()
        .tool()
        .run(PROGRAM, &["cwd".to_owned()], Some(30))
        .await
        .expect("the cwd dumper must run");
    assert_eq!(
        outcome.stdout.text.trim(),
        standard().workspace.display().to_string(),
        "the child's cwd is the run workspace"
    );
}

#[tokio::test]
async fn an_exit_code_is_data_not_an_error() {
    let outcome = standard()
        .tool()
        .run(PROGRAM, &["exit".to_owned(), "7".to_owned()], Some(30))
        .await
        .expect("a non-zero exit is an Ok outcome");
    assert_eq!(outcome.exit_code, Some(7));
    assert!(!outcome.killed_by_timeout);
}

#[tokio::test]
async fn a_requested_timeout_narrows_but_never_widens() {
    let error = standard()
        .tool()
        .run(PROGRAM, &["sleep".to_owned(), "1".to_owned()], Some(301))
        .await
        .expect_err("a request above the default is refused");
    assert!(
        matches!(
            &error,
            RunnerError::TimeoutExceedsDefault {
                requested: 301,
                default: 300
            }
        ),
        "the refusal must carry both numbers: {error}"
    );
    let outcome = standard()
        .tool()
        .run(PROGRAM, &["sleep".to_owned(), "1".to_owned()], Some(30))
        .await
        .expect("a request at or below the default narrows");
    assert_eq!(outcome.exit_code, Some(0));
}

/// The runner is absent where the probe did not prove the sandbox —
/// the M5-3 decision, consumed: no spawn configuration, no tool.
#[test]
fn the_runner_is_absent_where_the_probe_did_not_prove() {
    let workspace = leak("absent-ws");
    let programs = leak("absent-programs");
    let policy = RunSandbox::new([workspace.clone()], Vec::<(String, u16)>::new())
        .expect("the policy constructs while the root exists");
    fs::remove_dir_all(&workspace).expect("the root vanishes before prepare");
    let provision = policy.prepare(&programs).expect("preparation runs");
    assert!(!provision.report().proves_runner());
    assert!(provision.spawn().is_none(), "no spawn configuration exists");
}
