use super::*;

/// A granted token stops the ask: the second call of the same shape resolves
/// `Allow` with no prompt at all. Without the grant the same call still asks
/// (here, and asks deny when nobody can answer).
#[tokio::test]
async fn a_granted_token_stops_the_ask_without_a_prompt() {
    let tool = workspace_write_tool();
    let arguments = serde_json::json!({"path": "notes.md", "content": "hello"});
    let token = grant_token(&tool.name, &arguments, None, &workspace_rooted_facts())
        .expect("workspace_write is grantable");
    let policy = SessionPolicy::new(ApprovalPolicy::Ask);
    let before = TerminalApproval::from_session_with_cancellation(
        policy.clone(),
        false,
        TurnPrimary::default(),
        workspace_rooted_facts(),
        None,
        CancellationToken::new(),
    );
    assert!(
        !before.approve(&tool, &arguments).await,
        "an ungranted ask is decided by the mode: nobody to answer, so deny"
    );
    assert!(
        policy.record(ApprovalChoice::AllowSession { token }),
        "the first grant is new"
    );
    let after = TerminalApproval::from_session_with_cancellation(
        policy,
        false,
        TurnPrimary::default(),
        workspace_rooted_facts(),
        None,
        CancellationToken::new(),
    );
    assert!(
        after.approve(&tool, &arguments).await,
        "the granted token pre-answers the same shape with no prompt"
    );
}

/// "Allow once" leaves no grant behind: the store stays empty, so the same
/// call asks again. The answer's meaning is decided by the terminal's answer
/// mapping, the grant by the engine's own `record`.
#[test]
fn an_allow_once_answer_leaves_no_grant_behind() {
    let policy = SessionPolicy::new(ApprovalPolicy::Ask);
    let choice = terminal_choice("y", Some("workspace-write"));
    assert_eq!(choice, ApprovalChoice::AllowOnce, "`y` means allow once");
    assert!(!policy.record(choice), "allow once records no grant");
    assert!(policy.grants().is_empty(), "nothing was granted");
    assert_eq!(
        policy.resolve(&workspace_write_tool().effect, Some("workspace-write")),
        ApprovalDecision::Ask,
        "the same call asks again"
    );
}

/// The terminal's answer words keep their meaning: `y`/`yes` allow once and
/// `n`/`no` deny (a script or a habit must not break), `a` is the third
/// answer's allow-once, `s` grants the offered token only, `d` denies, and
/// anything unrecognised is a deny.
#[test]
fn the_terminal_answers_keep_their_meaning() {
    assert_eq!(terminal_choice("y", None), ApprovalChoice::AllowOnce);
    assert_eq!(terminal_choice("yes", None), ApprovalChoice::AllowOnce);
    assert_eq!(
        terminal_choice(" Y \n", None),
        ApprovalChoice::AllowOnce,
        "trimming and case are part of the habit"
    );
    assert_eq!(terminal_choice("n", None), ApprovalChoice::Deny);
    assert_eq!(terminal_choice("no", None), ApprovalChoice::Deny);
    assert_eq!(terminal_choice("d", None), ApprovalChoice::Deny);
    assert_eq!(terminal_choice("", None), ApprovalChoice::Deny);
    assert_eq!(
        terminal_choice("sure", None),
        ApprovalChoice::Deny,
        "anything unrecognised is a deny"
    );
    assert_eq!(
        terminal_choice("s", Some("runner:bench")),
        ApprovalChoice::AllowSession {
            token: "runner:bench".to_owned()
        },
        "`s` grants exactly the offered token"
    );
    assert_eq!(
        terminal_choice("s", None),
        ApprovalChoice::Deny,
        "`s` without a token is unoffered input: a deny, never a guess"
    );
}

/// The SQL family's grant rides the turn's primary: a call naming no
/// connection suggests `sql:<primary>` — the primary's real registry name —
/// and a session grant recorded for it pre-answers that connection's calls
/// while a different connection's call still asks. The ask is the decider's,
/// the grant the engine's own `record`.
#[tokio::test]
async fn the_sql_family_s_grant_rides_the_turn_s_primary() {
    let primary = TurnPrimary::default();
    primary.bind(&crate::grant_token_tests::registry_with_primary(
        "analytics",
    ));
    let policy = SessionPolicy::new(ApprovalPolicy::Ask);
    let sql = database_tools()
        .into_iter()
        .find(|tool| tool.name == "bounded_sql_query")
        .expect("bounded_sql_query is defined");
    let here = serde_json::json!({"sql": "SELECT 1"});
    assert_eq!(
        policy.resolve(&sql.effect, Some("sql:analytics")),
        ApprovalDecision::Ask,
        "the ungranted ask is decided by the mode"
    );
    assert!(
        policy.record(ApprovalChoice::AllowSession {
            token: "sql:analytics".to_owned()
        }),
        "the first grant is new"
    );
    let after = TerminalApproval::from_session_with_cancellation(
        policy.clone(),
        false,
        primary,
        ApprovalFacts::default(),
        None,
        CancellationToken::new(),
    );
    // The bound decider suggests `sql:analytics` for a connectionless call,
    // so the grant pre-answers it with no prompt at all.
    assert!(
        after.approve(&sql, &here).await,
        "the granted sql token pre-answers the primary's call with no prompt"
    );
    // A different connection suggests a different token, which is not
    // granted — the grant allows nothing on a different connection.
    assert_eq!(
        policy.resolve(&sql.effect, Some("sql:staging")),
        ApprovalDecision::Ask,
        "the grant allows nothing on a different connection"
    );
    let offered = approval_prompt(
        &sql,
        &here,
        Some("sql:analytics"),
        &ApprovalFacts::default(),
        None,
        None,
    );
    assert!(
        offered.contains("[s] allow sql:analytics for this session"),
        "the offered token is the primary's real name: {offered}"
    );
}

/// The prompt offers the session grant only when it can name the token: the
/// `[s]` answer appears with the token verbatim, and with no token the prompt
/// offers two answers and says so.
#[test]
fn the_prompt_offers_a_session_grant_only_when_one_exists() {
    let tool = workspace_write_tool();
    let arguments = serde_json::json!({"path": "notes.md", "content": "hello"});
    let token = grant_token(&tool.name, &arguments, None, &workspace_rooted_facts());
    let with = approval_prompt(
        &tool,
        &arguments,
        token.as_deref(),
        &workspace_rooted_facts(),
        None,
        None,
    );
    assert!(
        with.contains("[s] allow workspace-write for this session"),
        "the offered token is named verbatim: {with}"
    );
    assert!(
        with.contains("[a] allow once") && with.contains("[d] deny"),
        "the three answers are stated: {with}"
    );
    let none = approval_prompt(
        &tool,
        &arguments,
        None,
        &ApprovalFacts::default(),
        None,
        None,
    );
    assert!(
        !none.contains("[s]"),
        "no token, no session-grant offer: {none}"
    );
    assert!(
        none.contains("(no session grant for this tool)"),
        "the two-answer prompt says why the third is absent: {none}"
    );
}
