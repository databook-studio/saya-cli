/// The hard rule: every token the suggester produces parses under the
/// grammar **on the session surface** — the surface where grants live —
/// *and* approves the capability it names. Each case here is a shape the
/// suggester can produce; the parser (widened to `pub(crate)`) is the
/// authority it is judged against. (This round-trip previously judged
/// tokens against the run surface; the parse surface moved to the session
/// surface with the `sql:` family — that is where `/allow` seeds and
/// `SessionPolicy` consults the store.)
#[test]
fn every_suggestible_token_parses_to_the_capability_it_names() {
    let cases: Vec<(String, Value)> = vec![
        ("workspace_write".to_owned(), json!({})),
        (
            "workspace_write".to_owned(),
            json!({"path": "a.md", "content": "x"}),
        ),
        (
            "workspace_edit".to_owned(),
            json!({"path": "a.md", "old_text": "a", "new_text": "b", "expected_digest": "d"}),
        ),
        (
            "workspace_edit".to_owned(),
            json!({"path": "a.md", "offset": 0, "chunk": "x"}),
        ),
        (
            "scratch_sql".to_owned(),
            json!({"sql": "CREATE TABLE t (a int)"}),
        ),
        (
            "http_fetch".to_owned(),
            json!({"url": "https://a.example/x"}),
        ),
        (
            "http_download".to_owned(),
            json!({"url": "https://b.example/f.bin", "destination": "f.bin"}),
        ),
        ("run_program".to_owned(), json!({"program": "bench"})),
        ("run_program".to_owned(), json!({"program": "ripgrep"})),
        ("run_program".to_owned(), json!({"program": "python3"})),
        ("run_program".to_owned(), json!({"program": "bash"})),
        (
            "bounded_sql_query".to_owned(),
            json!({"sql": "SELECT 1", "connection": "analytics"}),
        ),
        (
            "result_shape".to_owned(),
            json!({"sql": "SELECT 1", "connection": "analytics"}),
        ),
        (
            "column_health".to_owned(),
            json!({"sql": "SELECT 1", "connection": "analytics"}),
        ),
        (
            "join_check".to_owned(),
            json!({"sql": "SELECT 1", "connection": "analytics"}),
        ),
    ];
    for (tool, arguments) in cases {
        let token = token_for(&tool, arguments, &composed_facts());
        let approved = scopes::parse(std::slice::from_ref(&token), scopes::Surface::Session)
            .unwrap_or_else(|error| panic!("`{token}` must parse under /allow: {error}"));
        assert!(
            approves_what_it_names(&token, &approved),
            "`{token}` must approve exactly the capability it names, got: {:?}",
            approved.capabilities
        );
    }
}

/// The session edit tool is the whole-file write's effect class: one
/// `workspace-write` grant covers both file tools, gated on the same bound
/// workspace root.
#[test]
fn workspace_edit_suggests_the_same_write_token_as_workspace_write() {
    let facts = composed_facts();
    let replace =
        json!({"path": "notes.md", "old_text": "a", "new_text": "b", "expected_digest": "d"});
    let append = json!({"path": "notes.md", "offset": 0, "chunk": "x"});
    let write = json!({"path": "notes.md", "content": "x"});
    assert_eq!(
        grant_token("workspace_edit", &replace, None, &facts),
        Some("workspace-write".to_owned()),
        "an anchored replace suggests the write token"
    );
    assert_eq!(
        grant_token("workspace_edit", &append, None, &facts),
        Some("workspace-write".to_owned()),
        "an append suggests the write token"
    );
    assert_eq!(
        grant_token("workspace_edit", &replace, None, &facts),
        grant_token("workspace_write", &write, None, &facts),
        "one grant covers both file tools"
    );
    assert_eq!(
        grant_token("workspace_edit", &replace, None, &ApprovalFacts::default()),
        None,
        "no workspace root, no offer — the same gate as the write tool"
    );
}

/// One SQL grant covers the read-shaped family: the family is derived from
/// the toolset's own declarations — a call that is one parsed read-only
/// statement against a *named* connection is
/// `external_side_effect: false`, `requires_approval: true`,
/// `local_state: None`, with a `sql` argument and a `connection` argument —
/// and the suggester's member list must be exactly the tools the
/// definitions declare that shape for. Keyed on the effect fields, not on
/// which block of the builder a definition happens to sit in.
#[test]
fn the_sql_family_is_the_definitions_read_sql_shape() {
    let definitions = DatabaseTools::definitions(true, false, false, false, true);
    let declared: Vec<&str> = definitions
        .iter()
        .filter(|tool| {
            let shaped_arguments = tool.parameters.get("properties").is_some_and(|properties| {
                properties.get("sql").is_some() && properties.get("connection").is_some()
            });
            !tool.effect.external_side_effect
                && tool.effect.requires_approval
                && tool.effect.local_state == LocalStateEffect::None
                && shaped_arguments
        })
        .map(|tool| tool.name.as_str())
        .collect();
    assert_eq!(
        declared, SQL_FAMILY,
        "the suggester's SQL family must be exactly the tools the definitions \
         declare as one read-only statement against a named connection"
    );
    // And each member suggests the same word for the same connection.
    let registry = registry_with_primary("analytics");
    for tool in SQL_FAMILY {
        assert_eq!(
            sql_suggestion(
                tool,
                json!({"sql": "SELECT 1", "connection": "staging"}),
                &registry
            ),
            Some("sql:staging".to_owned()),
            "every family member grants under one word per connection"
        );
    }
}

/// The grant is one word for one connection: `sql:analytics` pre-answers
/// every family member's call against `analytics` and nothing on a
/// different connection — a grant is additive only, so `sql:staging` stays
/// an ask under the same policy.
#[test]
fn one_sql_grant_covers_the_family_on_one_connection_only() {
    let definitions = DatabaseTools::definitions(true, false, false, false, true);
    let policy = SessionPolicy::new(ApprovalPolicy::Ask);
    policy.grants().grant("sql:analytics");
    for tool in SQL_FAMILY {
        let definition = definitions
            .iter()
            .find(|definition| definition.name == *tool)
            .unwrap_or_else(|| panic!("{tool} must be defined when the gate is open"));
        assert_eq!(
            policy.resolve(&definition.effect, Some("sql:analytics")),
            ApprovalDecision::Allow,
            "the one SQL grant pre-answers {tool} on the granted connection"
        );
        assert_eq!(
            policy.resolve(&definition.effect, Some("sql:staging")),
            ApprovalDecision::Ask,
            "the one SQL grant allows nothing on a different connection"
        );
    }
}

/// The fan-out tool (`bounded_sql_query_all`) suggests no token, and
/// `render_chart` — `external_side_effect: true`: it writes a file and
/// opens a browser, which `sql:<connection>`'s words do not say — keeps its
/// per-call allow-once. The fan-out's reason is its own: any "all" token's
/// referent can grow after approval (`/connect` mid-session puts a database
/// inside a grant made before it existed), so it is denied a token by
/// design, not by omission.
#[test]
fn render_chart_and_the_fan_out_suggest_no_token() {
    let registry = registry_with_primary("analytics");
    assert_eq!(
        grant_token(
            "bounded_sql_query_all",
            &json!({"sql": "SELECT 1"}),
            registry.primary(),
            &composed_facts()
        ),
        None,
        "the fan-out's referent can grow after approval — it suggests no token"
    );
    assert_eq!(
        grant_token(
            "render_chart",
            &json!({"sql": "SELECT 1", "chart_type": "bar", "connection": "analytics"}),
            registry.primary(),
            &composed_facts()
        ),
        None,
        "render_chart writes a file and opens a browser — not a `sql:` grant's \
         words"
    );
}
