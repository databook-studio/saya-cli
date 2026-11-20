//! Whose token a turn's budget stop latches: the caller's when nothing arms a
//! turn-scoped budget, the turn's own when a question scope does.

use super::*;
use saya_agent::{AgentRunControl, AnsweringScope, run_agent_with_control};

/// Nothing is armed, so the turn answers on the caller's own token: a spent
/// request budget latches there, which is the stop every caller read before a
/// turn could hold a budget of its own.
#[tokio::test]
async fn an_unarmed_budget_stop_latches_on_the_callers_token() {
    let run = CancellationToken::new();
    let sink = RecordingSink::new(run.clone());
    let (provider, requests) = ScriptedProvider::new(vec![turn("must not run", Vec::new())]);

    let output = controlled_run(
        &provider,
        &RecordingExecutor::default(),
        RunSettings {
            definitions: definitions(),
            limits: AgentLimits::default(),
            answering: answering_limits(Some(0), None, None),
        },
        &AllowReadOnlyApproval,
        &sink,
        run.clone(),
    )
    .await
    .expect("a spent question budget is a typed partial answer");

    assert!(stopped(&output, BudgetDimension::LogicalRequests));
    assert!(output.truncated);
    assert!(
        requests.lock().unwrap().is_empty(),
        "the stop lands before the request reaches the provider"
    );
    assert_eq!(
        run.reason(),
        Some(CancellationReason::Budget(BudgetDimension::LogicalRequests)),
        "the caller's token is what an unarmed budget stop latches"
    );
}

/// A question scope arms the turn's own budget: the same stop is the turn's,
/// and the caller's token never sees it.
#[tokio::test]
async fn an_armed_budget_stop_never_reaches_the_callers_token() {
    let run = CancellationToken::new();
    let scope = AnsweringScope::new(
        answering_limits(Some(0), None, None),
        CancellationToken::new(),
    );
    let sink = RecordingSink::new(run.clone());
    let (provider, requests) = ScriptedProvider::new(vec![turn("must not run", Vec::new())]);

    let output = run_agent_with_control(
        &provider,
        &RecordingExecutor::default(),
        request(),
        definitions(),
        AgentRunControl::new(
            AgentLimits::default(),
            answering_limits(Some(0), None, None),
            &AllowReadOnlyApproval,
            &sink,
            run.clone(),
        )
        .with_answering_scope(scope.clone()),
    )
    .await
    .expect("a spent question budget is a typed partial answer");

    assert!(stopped(&output, BudgetDimension::LogicalRequests));
    assert!(requests.lock().unwrap().is_empty());
    assert_eq!(
        scope.cancellation_token().reason(),
        Some(CancellationReason::Budget(BudgetDimension::LogicalRequests)),
        "the armed turn's own token is what its budget stop latches"
    );
    assert_eq!(
        run.reason(),
        None,
        "an armed turn keeps its budget stop out of the caller's token"
    );
}
