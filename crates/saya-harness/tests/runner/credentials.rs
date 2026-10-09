use super::*;

/// The parent's environment carries a planted database credential; the
/// child's ACTUAL environment — what it prints, not what was passed —
/// must not contain it.
#[tokio::test]
async fn a_planted_credential_is_absent_from_the_child_environment() {
    // One test plants into the process env; edition 2024 marks the
    // setter unsafe because it is a process-wide mutation.
    unsafe { std::env::set_var("SAYA_PROBE_DB_PASSWORD", "hunter2-planted") };
    let outcome = standard()
        .tool()
        .run(PROGRAM, &["env".to_owned()], Some(30))
        .await
        .expect("the env dumper must run");
    assert_eq!(outcome.exit_code, Some(0), "the child must have run");
    assert!(
        !outcome.stdout.text.contains("SAYA_PROBE_DB_PASSWORD"),
        "the planted credential's name must be absent from the child's actual env: {:?}",
        outcome.stdout.text
    );
    assert!(
        !outcome.stdout.text.contains("hunter2-planted"),
        "the planted credential's value must be absent from the child's actual env"
    );
}

/// Declared, sandboxed, referenced, redacted: the variable is in the
/// child's actual environment, and the value is scrubbed from the
/// captured output the model reads (the `SAYA_RUN_EP_*` env-assignment
/// rule in `redact()` — the generated name matches no `key=value`
/// marker).
#[tokio::test]
async fn the_declared_credential_reaches_the_sandboxed_child() {
    let outcome = credentialed_tool()
        .run(PROGRAM, &["env".to_owned()], Some(30))
        .await
        .expect("the run must complete");
    assert_eq!(outcome.exit_code, Some(0));
    assert!(
        outcome
            .stdout
            .text
            .contains(&format!("{}PROBE_ENDPOINT=", CREDENTIAL_ENV_PREFIX)),
        "the declared credential must reach the child's actual env: {:?}",
        outcome.stdout.text
    );
    assert!(
        !outcome.stdout.text.contains("sk-probe-value-123"),
        "the value itself must be scrubbed from captured output: {:?}",
        outcome.stdout.text
    );
}

/// Remove "declared in the approved plan": an undeclared credential is
/// never on the tool's list, so the variable vanishes.
#[tokio::test]
async fn an_undeclared_credential_never_reaches_the_child() {
    let outcome = standard()
        .tool_with_scope(RunnerScope::new(vec![PROGRAM.to_owned()]).expect("shaped"))
        .with_credentials(Vec::new())
        .run(PROGRAM, &["env".to_owned()], Some(30))
        .await
        .expect("the run must complete");
    assert!(
        !outcome
            .stdout
            .text
            .contains(&format!("{CREDENTIAL_ENV_PREFIX}PROBE_ENDPOINT")),
        "an undeclared credential must be absent from the child's env: {:?}",
        outcome.stdout.text
    );
}

/// Remove "references-only": a reference that cannot be resolved fails
/// the call before anything spawns — a declared credential that
/// silently did not inject would leave the child to fail in its place.
#[tokio::test]
async fn an_unresolvable_reference_injects_nothing_and_spawns_nothing() {
    let error = standard()
        .tool_with_scope(RunnerScope::new(vec![PROGRAM.to_owned()]).expect("shaped"))
        .with_credentials(vec![probe_credential()])
        .run(PROGRAM, &["env".to_owned()], Some(30))
        .await
        .expect_err("the unresolvable reference must fail the call");
    assert!(
        matches!(&error, RunnerError::CredentialUnresolved { credential, .. } if credential == "probe-endpoint"),
        "the failure must name the credential: {error}"
    );
}
