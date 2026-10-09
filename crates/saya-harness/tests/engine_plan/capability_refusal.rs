use super::*;

/// A plan asking for the workspace-write capability the run was never
/// approved for is rejected, re-prompted with what was wrong, and refused
/// with a typed code after the third attempt.
#[tokio::test]
async fn an_unapproved_capability_is_reprompted_then_refused_with_a_typed_code() {
    let plan_text = plan_json(&step_json(
        "write the report",
        r#"{"workspace_write": true}"#,
        None,
        None,
    ));
    let planner = ScriptedPlanner::new(vec![plan_text; 3]);
    let driver = PlanDriver::new(&planner, planner_request("produce the report"));

    let error = driver
        .propose(&Capabilities::default(), &Budgets::default())
        .await
        .unwrap_err();

    assert!(
        matches!(
            &error,
            PlanError::Exhausted {
                attempts: 3,
                last: PlanRejection::NeedsApproval { step: 0, scopes },
            } if scopes == &["workspace-write".to_string()]
        ),
        "the third refusal must be the typed needs-approval outcome, naming \
         what was missing: {error:?}"
    );

    // Exactly the bound was spent: three proposals, none more — the stub
    // panics on a fourth, so this also pins the cap.
    let requests = planner.requests();
    assert_eq!(requests.len(), 3, "three attempts, then the refusal");

    // The first ask carries no refusal; every re-prompt names what was wrong.
    assert!(
        !requests[0].messages[1].content.contains("REFUSAL"),
        "the first proposal must not carry a refusal"
    );
    for request in &requests[1..] {
        let user = &request.messages[1].content;
        assert!(
            user.contains("### REFUSAL OF THE PREVIOUS PLAN"),
            "a re-prompt must carry the previous refusal: {user}"
        );
        assert!(
            user.contains("outside the run's approved scopes"),
            "a re-prompt must name what was wrong: {user}"
        );
    }
}
