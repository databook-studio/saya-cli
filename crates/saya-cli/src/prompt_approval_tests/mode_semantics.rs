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

// The SQL family states its per-call facts rather than the generic
// "Allow bounded read-only SQL query?" sentence. The prompt's exact text is
// pinned by
// `approval_facts_tests::bounded_sql_query_prompt_pins_the_sql_family_s_facts`;
// the snapshot carries the reason: the generic SQL sentence could not say
// which connection or which bounds the call runs under.
