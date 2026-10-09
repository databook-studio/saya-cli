use crate::interactive::session::permissions::activation::SESSION_FORK_FACT;

/// `render_chart` says why it always asks: it writes a file and opens a
/// browser.
#[test]
fn render_chart_prompt_pins_why_it_always_asks() {
    let tool = database_tool("render_chart");
    let arguments = serde_json::json!(
        {"sql": "SELECT region, count(*) FROM orders GROUP BY 1", "connection": "analytics", "chart_type": "bar"}
    );
    let prompt = approval_prompt(
        &tool,
        &arguments,
        None,
        &sql_facts(),
        Some("analytics"),
        None,
    );
    insta::with_settings!({snapshot_path => "../snapshots"}, {
        insta::assert_snapshot!(prompt);
    });
}

/// `scratch_sql` states the statement and the scratch's own policy.
#[test]
fn scratch_sql_prompt_pins_the_scratch_facts() {
    let facts = session_facts();
    let tool = session_tool("scratch_sql");
    let arguments = serde_json::json!({"sql": "CREATE TABLE t AS SELECT 1"});
    let grant = grant_token(&tool.name, &arguments, None, &facts);
    let prompt = approval_prompt(&tool, &arguments, grant.as_deref(), &facts, None, None);
    insta::with_settings!({snapshot_path => "../snapshots"}, {
        insta::assert_snapshot!(prompt);
    });
}

/// `http_fetch` states the URL, the destination token a grant would record,
/// and the untrusted-block lane.
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

/// The `run_program` prompt pins the containment, then the argv.
#[test]
fn run_program_prompt_pins_the_program_s_facts() {
    let facts = session_facts();
    let tool = session_tool("run_program");
    let arguments =
        serde_json::json!({"program": "bench", "args": ["--json", "--out", "state/bench.json"]});
    let grant = grant_token(&tool.name, &arguments, None, &facts);
    let prompt = approval_prompt(&tool, &arguments, grant.as_deref(), &facts, None, None);
    insta::with_settings!({snapshot_path => "../snapshots"}, {
        insta::assert_snapshot!(prompt);
    });
}

/// The interpreter door's prompt carries the no-euphemism warning — the
/// session's own clause, the running platform's process-fork fact (U8:
/// the clause is per platform; the full body is snapshot-pinned on macOS,
/// where the committed snapshot lives, and the fork clause is pinned on
/// every platform by `the_fork_fact_says_only_what_the_running_platform_
/// enforces`).
#[test]
fn interpreter_run_program_prompt_carries_the_no_euphemism_warning() {
    let facts = session_facts();
    let tool = session_tool("run_program");
    let arguments = serde_json::json!({"program": "python3", "args": ["-c", "print(1)"]});
    let grant = grant_token(&tool.name, &arguments, None, &facts);
    let prompt = approval_prompt(&tool, &arguments, grant.as_deref(), &facts, None, None);
    // (Moved assertion, U8: the snapshot pins the macOS body — the fork
    // clause became the platform's own, so a non-macOS body differs in
    // exactly that clause and is pinned by the platform's own tests.)
    #[cfg(target_os = "macos")]
    insta::with_settings!({snapshot_path => "../snapshots"}, {
        insta::assert_snapshot!(prompt);
    });
    #[cfg(not(target_os = "macos"))]
    assert!(
        prompt.contains("interpreter approval: this session may execute python3"),
        "the warning's body renders: {prompt}"
    );
    assert!(
        prompt.contains(SESSION_FORK_FACT),
        "the session's fork clause for this platform, not the run's \
         conditional: {prompt}"
    );
    assert!(
        !prompt.contains("where process-fork is granted"),
        "the run surface's parenthetical is a run's clause: {prompt}"
    );
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

/// `workspace_write` states path, bytes, and the containment rule.
#[test]
fn workspace_write_prompt_pins_the_containment_facts() {
    let facts = session_facts();
    let tool = session_tool("workspace_write");
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
