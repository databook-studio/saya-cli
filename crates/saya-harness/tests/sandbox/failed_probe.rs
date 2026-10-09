use super::*;

/// Test 1 — the fail-closed chain: probe failure ⇒ the runner is not
/// registered (no spawn configuration exists, so the tool universe carries
/// no runner tool) ⇒ a plan requesting the runner scope is refused with the
/// ordinary needs-approval outcome after the bounded attempts.
#[cfg(unix)]
#[tokio::test]
async fn probe_failure_leaves_the_runner_unregistered_and_refuses_runner_scopes() {
    let (policy, _root) = policy_with_vanished_root();
    let provision = policy
        .prepare(&runner_program_dir())
        .expect("preparation itself must run");

    // The probe ran and did not prove the runner.
    let report = provision.report();
    assert!(
        !report.proves_runner(),
        "a policy whose roots vanished must not be proven:\n{}",
        report.render()
    );
    assert!(
        !report.failed_required().is_empty(),
        "the report must name what failed:\n{}",
        report.render()
    );

    // The tool list: with no spawn configuration the runner tool cannot join
    // the universe the composition root builds.
    assert!(
        runner_tool_names(&provision).is_empty(),
        "an unproven sandbox registers no runner tool"
    );
    assert!(provision.spawn().is_none());

    // The plan surface: a plan requesting the runner scope is refused with
    // the ordinary needs-approval outcome, after the bounded re-prompts.
    let approved = approved_runner_scopes();
    let stripped = provision.plan_capabilities(&approved);
    assert!(
        stripped.runner.is_none(),
        "the runner scope must be absent from what plan validation sees"
    );

    let planner = ScriptedPlanner::new(vec![runner_plan_json(); 3]);
    let driver = PlanDriver::new(
        &planner,
        PlanRequest {
            model: "mock-model".into(),
            run_goal: "run the approved program".into(),
        },
    );
    let error = driver
        .propose(&stripped, &Budgets::default())
        .await
        .unwrap_err();
    assert!(
        matches!(
            &error,
            PlanError::Exhausted {
                attempts: 3,
                last: PlanRejection::NeedsApproval { step: 0, scopes: _ },
            }
        ),
        "the third refusal must be the typed needs-approval outcome: {error:?}"
    );
    assert_eq!(
        planner.request_count(),
        3,
        "exactly the bound was spent: three proposals, then the refusal"
    );
}
