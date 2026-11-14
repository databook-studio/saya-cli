/// `http_fetch` states the URL, the destination token a grant would record,
/// and the untrusted-block route.
#[test]
fn http_fetch_prompt_pins_the_destination_and_the_untrusted_lane() {
    let facts = session_facts();
    let tool = session_tool("http_fetch");
    let arguments = serde_json::json!({"url": "https://api.github.com/repos/x/y"});
    let grant = grant_token(&tool.name, &arguments, None, &facts);
    let prompt = approval_prompt(&tool, &arguments, grant.as_deref(), &facts, None, None);
    insta::with_settings!({snapshot_path => "../snapshots"}, {
        insta::assert_snapshot!(prompt);
    });
}

/// `http_download` states the URL, the target path, and the *remaining*
/// download budget.
#[test]
fn http_download_prompt_pins_the_remaining_budget() {
    let facts = session_facts();
    let tool = session_tool("http_download");
    let arguments = serde_json::json!({"url": "https://example.com/model.bin", "destination": "artifacts/model.bin"});
    let grant = grant_token(&tool.name, &arguments, None, &facts);
    let prompt = approval_prompt(&tool, &arguments, grant.as_deref(), &facts, None, None);
    insta::with_settings!({snapshot_path => "../snapshots"}, {
        insta::assert_snapshot!(prompt);
    });
}

/// The SQL family's prompt states the enforced bounds as facts.
#[test]
fn bounded_sql_query_prompt_pins_the_sql_family_s_facts() {
    let tool = database_tool("bounded_sql_query");
    let arguments = serde_json::json!(
        {"sql": "SELECT region, count(*) FROM orders GROUP BY 1", "connection": "analytics"}
    );
    let grant = grant_token(&tool.name, &arguments, None, &sql_facts());
    let prompt = approval_prompt(
        &tool,
        &arguments,
        grant.as_deref(),
        &sql_facts(),
        Some("analytics"),
        None,
    );
    insta::with_settings!({snapshot_path => "../snapshots"}, {
        insta::assert_snapshot!(prompt);
    });
}

/// `write` states path, bytes, and the containment rule.
#[test]
fn workspace_write_prompt_pins_the_containment_facts() {
    let facts = session_facts();
    let tool = session_tool("write");
    let arguments = serde_json::json!({"path": "notes/summary.md", "content": "hello world"});
    let grant = grant_token(&tool.name, &arguments, None, &facts);
    let prompt = approval_prompt(&tool, &arguments, grant.as_deref(), &facts, None, None);
    insta::with_settings!({snapshot_path => "../snapshots"}, {
        insta::assert_snapshot!(prompt);
    });
}

/// A twentieth-prompt shape: the session's held grants are stated, with the
/// allowed-call count, so a fresh ask reads as "still inside what you
/// approved" rather than as a fresh ask.
#[test]
fn the_session_line_shows_the_family_s_held_grants_and_call_counts() {
    let policy = SessionPolicy::new(ApprovalPolicy::Ask);
    assert!(policy.record(ApprovalChoice::AllowSession {
        token: "sql:analytics".to_owned(),
    }));
    for _ in 0..3 {
        assert_eq!(
            policy.resolve(
                &database_tool("bounded_sql_query").effect,
                Some("sql:analytics")
            ),
            ApprovalDecision::Allow
        );
    }
    let tool = database_tool("bounded_sql_query");
    let arguments = serde_json::json!({"sql": "SELECT 1", "connection": "warehouse"});
    let prompt = approval_prompt(
        &tool,
        &arguments,
        Some("sql:warehouse"),
        &sql_facts(),
        None,
        Some(policy.grants()),
    );
    insta::with_settings!({snapshot_path => "../snapshots"}, {
        insta::assert_snapshot!(prompt);
    });
}
