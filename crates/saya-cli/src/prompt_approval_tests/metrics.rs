use super::*;

/// The `approval_wait` phase cell of a collector's snapshot.
fn approval_wait_cell(metrics: &Arc<LocalMetrics>) -> PhaseCell {
    let snapshot = metrics.snapshot();
    let index = phase_index(AgentMetricPhase::ApprovalWait).expect("approval_wait is indexed");
    snapshot.phases[index]
}

/// An automatic decision never enters the real wait: the mode resolves (or
/// the nobody-to-answer deny lands) before the terminal read, so no
/// ApprovalWait sample is recorded — the section keeps printing
/// `unavailable`, never a successful zero.
#[tokio::test]
async fn an_automatic_decision_records_no_approval_wait_sample() {
    let metrics = Arc::new(LocalMetrics::new());
    let sql = database_tools()
        .into_iter()
        .find(|tool| tool.name == "bounded_sql_query")
        .expect("bounded_sql_query is defined");
    let auto_allow = TerminalApproval::with_cancellation(
        ApprovalPolicy::ReadOnly,
        false,
        TurnPrimary::default(),
        ApprovalFacts::default(),
        CancellationToken::new(),
    )
    .with_metrics(Some(metrics.clone()));
    assert!(
        auto_allow
            .approve(&sql, &serde_json::json!({"sql": "SELECT 1"}))
            .await,
        "read-only auto-approves the read-shaped SQL tool with no wait"
    );
    let auto_deny = TerminalApproval::with_cancellation(
        ApprovalPolicy::Ask,
        false,
        TurnPrimary::default(),
        ApprovalFacts::default(),
        CancellationToken::new(),
    )
    .with_metrics(Some(metrics.clone()));
    assert!(
        !auto_deny
            .approve(&side_effecting_tool(), &serde_json::json!({}))
            .await,
        "an ask nobody can answer denies with no wait"
    );
    assert_eq!(
        approval_wait_cell(&metrics).samples,
        0,
        "an automatic decision enters no wait, so records no sample"
    );
}

/// A real wait ended by its own cancellation token records ONE ApprovalWait
/// sample with the Cancelled outcome: the wait began, the token ended it,
/// and the deny is the cancelled wait's deny — never a success.
#[tokio::test]
async fn a_cancelled_wait_records_one_cancelled_sample() {
    let metrics = Arc::new(LocalMetrics::new());
    let token = CancellationToken::new();
    token.cancel();
    let approval = TerminalApproval::with_cancellation(
        ApprovalPolicy::Ask,
        true,
        TurnPrimary::default(),
        ApprovalFacts::default(),
        token,
    )
    .with_metrics(Some(metrics.clone()));
    assert!(
        !approval
            .approve(&side_effecting_tool(), &serde_json::json!({}))
            .await,
        "a wait ended by its token denies"
    );
    let cell = approval_wait_cell(&metrics);
    assert_eq!(cell.samples, 1, "the wait began, so exactly one sample");
    assert_eq!(
        cell.outcomes[outcome_index(AgentMetricOutcome::Cancelled).expect("cancelled is indexed")],
        1,
        "the token ended the wait: the sample is a cancellation"
    );
    assert_eq!(
        cell.outcomes[outcome_index(AgentMetricOutcome::Succeeded).expect("succeeded is indexed")],
        0,
        "a cancelled wait is never a success"
    );
}
