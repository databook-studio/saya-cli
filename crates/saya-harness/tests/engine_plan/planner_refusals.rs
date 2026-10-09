use super::*;

/// A model returning prose, truncated JSON, or the wrong shape is an
/// ordinary typed failure — never a panic.
#[tokio::test]
async fn prose_truncated_and_wrong_shaped_answers_are_typed_failures() {
    let cases = [
        ("I will draft the plan shortly.", PlanParseFailure::Prose),
        (r#"{"steps": [{"goal": "read"#, PlanParseFailure::Truncated),
        (
            r#"{"steps": "two, trust me"}"#,
            PlanParseFailure::WrongShape,
        ),
    ];
    for (answer, expected) in cases {
        let planner = ScriptedPlanner::new(vec![answer.to_string(); 3]);
        let driver = PlanDriver::new(&planner, planner_request("g"));

        let error = driver
            .propose(&Capabilities::default(), &Budgets::default())
            .await
            .unwrap_err();

        assert!(
            matches!(
                &error,
                PlanError::Exhausted {
                    attempts: 3,
                    last: PlanRejection::Malformed { kind },
                } if *kind == expected
            ),
            "`{answer}` must be an ordinary typed failure: {error:?}"
        );

        // The plan call asks for JSON mode; the effort lever stays with the
        // endpoint — a provider that cannot honour an effort variant drops
        // it rather than erroring, so the call must not depend on it.
        let sent = &planner.requests()[0];
        assert_eq!(sent.response_format, ResponseFormat::JsonObject);
        assert_eq!(sent.reasoning_effort, ReasoningEffort::Default);
    }
}

/// A step naming an endpoint role the run has no binding for is refused.
#[tokio::test]
async fn an_endpoint_role_the_run_does_not_bind_is_refused() {
    let mut approval = Capabilities::default();
    approval.endpoints = EndpointBindings::new([("orchestrator", "primary")]).unwrap();
    let plan_text = plan_json(&step_json(
        "ask the shadow endpoint",
        r#"{"endpoints": {"orchestrator": "primary"}}"#,
        None,
        Some("shadow"),
    ));
    let planner = ScriptedPlanner::new(vec![plan_text; 3]);
    let driver = PlanDriver::new(&planner, planner_request("g"));

    let error = driver
        .propose(&approval, &Budgets::default())
        .await
        .unwrap_err();

    assert!(
        matches!(
            &error,
            PlanError::Exhausted {
                attempts: 3,
                last: PlanRejection::EndpointUnbound { step: 0, role },
            } if role == "shadow"
        ),
        "the unbound role must be refused by name: {error:?}"
    );
}

/// A plan proposing a new capability mid-run — at a re-plan boundary, against
/// the budget that is left — yields the needs-approval outcome rather than
/// running.
#[tokio::test]
async fn a_new_capability_mid_run_yields_the_needs_approval_outcome_rather_than_running() {
    let plan_text = plan_json(&format!(
        "{},{}",
        step_json("read the manifest", "{}", None, None),
        step_json(
            "seed the scratch database",
            r#"{"scratch": true}"#,
            None,
            None
        )
    ));
    let planner = ScriptedPlanner::new(vec![plan_text; 3]);
    let driver = PlanDriver::new(&planner, planner_request("seed and report"));
    let mut remaining = Budgets::default();
    remaining.turns = Some(3);

    let error = driver
        .propose(&Capabilities::default(), &remaining)
        .await
        .unwrap_err();

    assert!(
        matches!(
            &error,
            PlanError::Exhausted {
                attempts: 3,
                last: PlanRejection::NeedsApproval { step: 1, scopes },
            } if scopes == &["scratch".to_string()]
        ),
        "the new capability must be the needs-approval outcome, pointed at \
         its step and named: {error:?}"
    );
    // The refusal is the outcome: no plan bound, so nothing ran.
    assert_eq!(planner.requests().len(), 3);
}
