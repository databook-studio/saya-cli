use super::*;

/// The run's decider (U4): `TerminalApproval::frozen`, seeded from the run's
/// `--allow`. A `sql:` seed pre-answers exactly the SQL calls that name the
/// seeded connection, so a run started with `--allow sql:analytics` behaves
/// differently from one without it: the seeded decider runs the call the
/// unseeded one denies. The suggester is the run's own (the run names its
/// connection in the call; the primary stays unbound, so nothing is guessed).
#[tokio::test]
async fn a_run_started_with_allow_sql_gates_the_sql_family_s_asks() {
    let sql = database_tools()
        .into_iter()
        .find(|tool| tool.name == "bounded_sql_query")
        .expect("bounded_sql_query is defined");
    let call = serde_json::json!({"sql": "SELECT 1", "connection": "analytics"});
    let token = grant_token(&sql.name, &call, None, &ApprovalFacts::default())
        .expect("the call names its connection");
    assert_eq!(token, "sql:analytics", "the token is the call's connection");

    let granted = TerminalApproval::frozen(
        ApprovalPolicy::Ask,
        &["sql:analytics".to_owned()],
        ApprovalFacts::default(),
    );
    assert!(
        granted.approve(&sql, &call).await,
        "the seeded `--allow sql:analytics` pre-answers the connection's call"
    );
    let unseeded = TerminalApproval::frozen(ApprovalPolicy::Ask, &[], ApprovalFacts::default());
    assert!(
        !unseeded.approve(&sql, &call).await,
        "without the seed, the same call on a headless run denies"
    );
    // A different connection is outside the seed: the grant is narrow.
    let other = serde_json::json!({"sql": "SELECT 1", "connection": "staging"});
    assert!(
        !granted.approve(&sql, &other).await,
        "the seed covers only the connection it names"
    );
}

/// The run's frozen decider cannot accumulate: whatever the mode, no ask is
/// answerable and no answer path exists — two calls of the same ungranted
/// shape both deny, and no grant could have been recorded between them. The
/// engine's own pin (`a_frozen_policy_cannot_accumulate_a_grant`) holds the
/// store-side proof; this pins the decider the run composes.
#[tokio::test]
async fn the_run_s_frozen_decider_cannot_accumulate() {
    let tool = workspace_write_tool();
    let arguments = serde_json::json!({"path": "notes.md", "content": "hello"});
    let token = grant_token(&tool.name, &arguments, None, &workspace_rooted_facts())
        .expect("workspace_write is grantable");
    let decider = TerminalApproval::frozen(ApprovalPolicy::Ask, &[], workspace_rooted_facts());
    let first = decider.approve(&tool, &arguments).await;
    let second = decider.approve(&tool, &arguments).await;
    assert!(!first && !second, "an ungranted headless ask denies, twice");
    let with_seed =
        TerminalApproval::frozen(ApprovalPolicy::Ask, &[token], workspace_rooted_facts());
    assert!(
        with_seed.approve(&tool, &arguments).await,
        "the seed pre-answers, and the seeds are the only grants a run holds"
    );
}

/// A run's frozen decider under the modes that never ask is unchanged:
/// read-only still auto-approves the read-shaped SQL tools and denies the
/// side-effecting ones; `never` denies everything. Seeds ride along inert.
#[tokio::test]
async fn the_run_s_frozen_decider_under_read_only_and_never() {
    let sql = database_tools()
        .into_iter()
        .find(|tool| tool.name == "bounded_sql_query")
        .expect("bounded_sql_query is defined");
    let seeds = ["sql:analytics".to_owned()];
    let read_only =
        TerminalApproval::frozen(ApprovalPolicy::ReadOnly, &seeds, ApprovalFacts::default());
    assert!(
        read_only
            .approve(&sql, &serde_json::json!({"sql": "SELECT 1"}))
            .await,
        "read-only auto-approves the read-shaped SQL tools as before"
    );
    let render_chart = database_tools()
        .into_iter()
        .find(|tool| tool.name == "render_chart")
        .expect("render_chart is defined");
    assert!(
        !read_only
            .approve(
                &render_chart,
                &serde_json::json!({"sql": "SELECT 1", "chart_type": "bar"})
            )
            .await,
        "read-only denies the side-effecting tool; no seed can move it"
    );
    let never = TerminalApproval::frozen(ApprovalPolicy::Never, &seeds, ApprovalFacts::default());
    assert!(
        !never
            .approve(
                &sql,
                &serde_json::json!({"sql": "SELECT 1", "connection": "analytics"})
            )
            .await,
        "never denies everything, seed or not"
    );
}
