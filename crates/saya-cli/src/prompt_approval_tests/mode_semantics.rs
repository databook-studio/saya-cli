use super::*;

#[tokio::test]
async fn read_only_approval_denies_a_side_effecting_tool() {
    let approval = TerminalApproval::with_cancellation(
        ApprovalPolicy::ReadOnly,
        false,
        TurnPrimary::default(),
        ApprovalFacts::default(),
        CancellationToken::new(),
    );
    assert!(
        !approval
            .approve(&side_effecting_tool(), &serde_json::json!({}))
            .await,
        "read-only must not auto-approve a tool with an external side effect"
    );
}

#[tokio::test]
async fn non_interactive_read_only_denies_render_chart_and_still_approves_sql() {
    let approval = TerminalApproval::with_cancellation(
        ApprovalPolicy::ReadOnly,
        false,
        TurnPrimary::default(),
        ApprovalFacts::default(),
        CancellationToken::new(),
    );
    let tools = database_tools();
    let render_chart = tools
        .iter()
        .find(|tool| tool.name == "render_chart")
        .expect("render_chart is defined");
    assert!(
        !approval
            .approve(
                render_chart,
                &serde_json::json!({"sql": "SELECT 1", "chart_type": "bar"})
            )
            .await,
        "render_chart writes a file and spawns a browser; read-only must deny it"
    );
    let sql = tools
        .iter()
        .find(|tool| tool.name == "bounded_sql_query")
        .expect("bounded_sql_query is defined");
    assert!(
        approval
            .approve(sql, &serde_json::json!({"sql": "SELECT 1"}))
            .await,
        "the SQL tools must stay auto-approved under read-only"
    );
}

/// The arms the read-only change must not touch: `never` denies, and `ask`
/// without a terminal to prompt on denies — no matter how read-shaped the tool
/// is.
#[tokio::test]
async fn never_denies_and_ask_without_a_terminal_denies() {
    let tools = database_tools();
    let sql = tools
        .iter()
        .find(|tool| tool.name == "bounded_sql_query")
        .expect("bounded_sql_query is defined");
    let never = TerminalApproval::with_cancellation(
        ApprovalPolicy::Never,
        false,
        TurnPrimary::default(),
        ApprovalFacts::default(),
        CancellationToken::new(),
    );
    assert!(
        !never
            .approve(sql, &serde_json::json!({"sql": "SELECT 1"}))
            .await
    );
    let ask = TerminalApproval::with_cancellation(
        ApprovalPolicy::Ask,
        false,
        TurnPrimary::default(),
        ApprovalFacts::default(),
        CancellationToken::new(),
    );
    assert!(
        !ask.approve(sql, &serde_json::json!({"sql": "SELECT 1"}))
            .await
    );
}

/// A tool whose call has no visible detail (`tool_call_detail` is `None`) is
/// prompted with a generic sentence naming the tool — never with the SQL
/// sentence it does not match.
#[test]
fn ask_prompt_uses_the_generic_sentence_for_non_sql_tools() {
    let tools = database_tools();
    let designate = tools
        .iter()
        .find(|tool| tool.name == "designate_answer")
        .expect("designate_answer is defined");
    let prompt = approval_prompt(
        designate,
        &serde_json::json!({"sql": "SELECT 1"}),
        None,
        &ApprovalFacts::default(),
        None,
        None,
    );
    assert!(
        prompt.contains("Run tool `designate_answer`"),
        "a tool with no visible detail gets the generic sentence: got \"{prompt}\""
    );
    assert!(
        !prompt.contains("read-only SQL"),
        "the SQL sentence must not appear for a tool it does not describe: got \"{prompt}\""
    );
}

// Moved assertion (U5): the old byte-pinned "Allow bounded read-only SQL
// query?" sentence is replaced by the SQL family's per-call fact lines. The
// prompt's exact text is now pinned by
// `approval_facts_tests::bounded_sql_query_prompt_pins_the_sql_family_s_facts`
// — the snapshot is this assertion's new home, with its reason: the generic
// SQL sentence could not say which connection or which bounds the call runs
// under.
