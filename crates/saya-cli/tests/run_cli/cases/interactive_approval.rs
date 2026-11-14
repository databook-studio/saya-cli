/// Interactive approval of the plan satisfies it and the run proceeds: the
/// approval view is shown once, an explicit yes approves, and the run
/// completes — with no per-tool-call prompt afterwards.
#[test]
fn interactive_approval_of_the_plan_satisfies_it_and_the_run_proceeds() {
    let env = test_root("interactive-approval");
    let (address, _ready) = mock(vec![
        Scripted {
            body: plan_body(&["survey the data quality"]),
            delay_ms: 0,
        },
        Scripted {
            body: sse("survey complete"),
            delay_ms: 0,
        },
    ]);
    let run = run_interactively(
        &env,
        &[
            "run",
            "--allow",
            "workspace-write",
            "survey the data quality",
        ],
        &address,
        "y\n",
    );
    assert_eq!(
        run.exit_code,
        Some(0),
        "the approved run must complete; stream:\n{}",
        run.stream
    );
    // The approval view named what was being approved.
    assert!(
        run.stream.contains("survey the data quality"),
        "the view shows the plan: {}",
        run.stream
    );
    assert!(
        run.stream.contains("approved scopes: workspace-write"),
        "the view shows the granted scopes: {}",
        run.stream
    );
    // The one approval is on the durable record, and the run completed.
    let journal = only_run_journal(&env);
    assert_eq!(
        journal.matches("plan_approved").count(),
        1,
        "exactly one approval interaction: {journal}"
    );
    assert!(
        journal.contains("\"completed\""),
        "the approved run proceeded to completion: {journal}"
    );
    let _ = fs::remove_dir_all(&env.root);
}

/// A refusal at the approval gate refuses the run: exit 2, the run stays
/// unapproved, and nothing executed.
#[test]
fn refusing_the_plan_at_the_approval_gate_refuses_the_run() {
    let env = test_root("interactive-deny");
    let (address, _ready) = mock(vec![
        Scripted {
            body: plan_body(&["survey the data quality"]),
            delay_ms: 0,
        },
        Scripted {
            body: sse("never reached"),
            delay_ms: 0,
        },
    ]);
    let run = run_interactively(
        &env,
        &[
            "run",
            "--allow",
            "workspace-write",
            "survey the data quality",
        ],
        &address,
        "n\n",
    );
    assert_eq!(
        run.exit_code,
        Some(2),
        "a refused plan is a usage refusal; stream:\n{}",
        run.stream
    );
    assert!(
        run.stream.contains("was not approved"),
        "the refusal must say why: {}",
        run.stream
    );
    let journal = only_run_journal(&env);
    assert!(
        !journal.contains("\"completed\"") && !journal.contains("step_started"),
        "a refused run executed nothing: {journal}"
    );
    let _ = fs::remove_dir_all(&env.root);
}
