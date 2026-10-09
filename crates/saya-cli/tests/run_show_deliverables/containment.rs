/// A deliverable naming a path outside the workspace is refused: the plan
/// never binds, the run stops loudly, and the outside sentinel is neither
/// read nor listed anywhere.
#[test]
fn a_deliverable_naming_a_path_outside_the_workspace_is_refused() {
    let env = test_root("outside");
    let sentinel = env.root.join("sentinel.txt");
    fs::write(&sentinel, OUTSIDE_CONTENT).unwrap();
    // The model proposes the same escaping plan on every attempt; the bound
    // is spent and the run stops without binding it.
    let escaping_plan = plan_with_expects(&[("../sentinel.txt", "the outside file")]);
    let (address, _ready) = mock(vec![
        Scripted {
            body: escaping_plan.clone(),
            delay_ms: 0,
        },
        Scripted {
            body: escaping_plan.clone(),
            delay_ms: 0,
        },
        Scripted {
            body: escaping_plan,
            delay_ms: 0,
        },
    ]);
    let started = saya(
        &env,
        &[
            "--non-interactive",
            "run",
            "--allow",
            "workspace-write",
            "produce the outside file",
        ],
        &address,
    );
    assert_eq!(
        started.status.code(),
        Some(5),
        "an escaping deliverable must refuse the plan; stderr: {}",
        stderr(&started)
    );
    assert!(
        stderr(&started).contains("could not bind a plan"),
        "the refusal must say the plan never bound: {}",
        stderr(&started)
    );
    assert!(
        stderr(&started).contains("expected output"),
        "the refusal must name the offending expectation: {}",
        stderr(&started)
    );
    // The sentinel is intact — never read for a digest, never written.
    assert_eq!(
        fs::read_to_string(&sentinel).unwrap(),
        OUTSIDE_CONTENT,
        "the outside sentinel must be untouched"
    );
    // And it is listed nowhere: the run's own show has no deliverables.
    let ids = listed_ids(&env, &address);
    assert_eq!(ids.len(), 1, "exactly the refused run must exist");
    let shown = stdout(&show(&env, &ids[0], &address));
    assert!(
        !shown.contains("sentinel"),
        "the outside path must never be listed: {shown}"
    );
    let _ = fs::remove_dir_all(&env.root);
}
