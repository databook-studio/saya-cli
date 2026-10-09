/// A call naming no connection suggests the primary's **real registry
/// name** — never a magic word like `sql:default`, so `/grants` shows the
/// database the user actually approved. With no primary resolved (the
/// turn built no registry, or the registry is empty), the suggestion is
/// `None` — never a guessed name.
#[test]
fn a_call_naming_no_connection_suggests_the_primary_s_real_name() {
    let arguments = json!({"sql": "SELECT 1"});
    let registry = registry_with_primary("analytics");
    for tool in SQL_FAMILY {
        assert_eq!(
            sql_suggestion(tool, arguments.clone(), &registry),
            Some("sql:analytics".to_owned()),
            "{tool} with no connection names the primary's registry name"
        );
    }
    // The registry's own rule: an empty connection string IS the primary.
    assert_eq!(
        sql_suggestion(
            "bounded_sql_query",
            json!({"sql": "SELECT 1", "connection": ""}),
            &registry
        ),
        Some("sql:analytics".to_owned()),
        "an empty connection names no connection — the primary's own rule"
    );
    // No connection in the registry, no token.
    assert_eq!(
        sql_suggestion("bounded_sql_query", arguments.clone(), &empty_registry()),
        None,
        "no primary resolves, no token — never a guessed name"
    );
    // A primary whose name fails the grammar's shape rule is never spelled.
    let mut odd = ConnectionRegistry::new("prod eu");
    odd.insert(
        "prod eu",
        crate::connection::ConnectionEntry {
            connector: Box::new(DummyConnector {
                dialect: SqlDialect::Postgres,
            }),
            dialect: SqlDialect::Postgres,
            profile_id: None,
        },
    );
    assert_eq!(
        sql_suggestion("bounded_sql_query", arguments, &odd),
        None,
        "a non-name-shaped primary is never a grant word"
    );
}

/// A named connection is judged by the same shape rule `endpoint:` payloads
/// are: non-empty, bounded, no whitespace, no control characters. A name
/// that fails the shape yields `None` — never a guessed token.
#[test]
fn a_named_connection_is_judged_by_the_name_shape_rule() {
    let registry = registry_with_primary("analytics");
    assert_eq!(
        sql_suggestion(
            "bounded_sql_query",
            json!({"sql": "SELECT 1", "connection": "bad name"}),
            &registry
        ),
        None,
        "whitespace is never a connection name"
    );
    assert_eq!(
        sql_suggestion(
            "bounded_sql_query",
            json!({"sql": "SELECT 1", "connection": 7}),
            &registry
        ),
        None,
        "a non-string connection is not a name"
    );
    assert_eq!(
        sql_suggestion(
            "bounded_sql_query",
            json!({"sql": "SELECT 1", "connection": "Analytics_2"}),
            &registry
        ),
        Some("sql:Analytics_2".to_owned()),
        "a name-shaped payload rides verbatim"
    );
}

/// The privacy gate is above grants, never beside them: with
/// `allow_query_data` off, the whole SQL family is hidden — no definition,
/// so no ask a grant could pre-answer. A grant in the store cannot re-open
/// the gate: the toolset builder takes no grants, so a store holding
/// `sql:analytics` advertises nothing new.
#[test]
fn the_privacy_gate_stands_above_the_grants() {
    let closed = DatabaseTools::definitions(false, false, false, false, false);
    let open = DatabaseTools::definitions(true, false, false, false, true);
    for tool in SQL_FAMILY {
        assert!(
            !closed.iter().any(|definition| definition.name == *tool),
            "the gate is closed: `{tool}` must be hidden, so no ask exists a \
             grant could pre-answer"
        );
        assert!(
            open.iter().any(|definition| definition.name == *tool),
            "the gate open: `{tool}` is the family this slice grants"
        );
    }
    let granted = SessionPolicy::new(ApprovalPolicy::Ask);
    granted.grants().grant("sql:analytics");
    assert!(!granted.grants().is_empty(), "the store holds a grant");
    for tool in SQL_FAMILY {
        assert!(
            !closed.iter().any(|definition| definition.name == *tool),
            "`{tool}` stays hidden with a grant in the store — a grant cannot \
             re-open the gate"
        );
    }
}

/// The tools this slice does not grant get `None`, which means they keep
/// asking every call: tools with `requires_approval: false` are never
/// asked (nothing to grant), the non-SQL file tools name no grantable
/// shape, and an unknown tool is not a grant either — with the turn's
/// primary bound or not.
#[test]
fn tools_outside_the_grammar_s_families_get_no_token() {
    let registry = registry_with_primary("analytics");
    for tool in [
        "schema_discovery",
        "workspace_read",
        "workspace_list",
        "glob",
        "grep",
        "designate_answer",
        "contract_search",
        "contract_read",
        "no_such_tool",
    ] {
        assert_eq!(
            grant_token(
                tool,
                &json!({"sql": "SELECT 1"}),
                registry.primary(),
                &composed_facts()
            ),
            None,
            "{tool} must keep asking every call — no token names it"
        );
    }
}

/// The fetch token's host spelling is the run engine's, not a private one:
/// the engine's fetch policy compares `FetchDestination::new(url.scheme(),
/// url.host_str())` — lowercased, host-only, no port — so the token must be
/// spelled exactly that way or a granted token would never match a fetched
/// destination.
#[test]
fn the_fetch_token_spells_the_host_the_run_engine_s_way() {
    for url in [
        "https://a.example/x",
        "https://A.Example/deep/path?q=1",
        "https://a.example:8443/x",
        "https://a.example",
    ] {
        let token = token_for("http_fetch", json!({"url": url}), &composed_facts());
        let parsed = url::Url::parse(url).expect("the test URL parses");
        let engine = FetchDestination::new(
            parsed.scheme(),
            parsed.host_str().expect("the test URL names a host"),
        );
        assert_eq!(
            token,
            format!("fetch:{}+{}", engine.scheme(), engine.host()),
            "the token must name the destination as the run engine normalises it"
        );
    }
}
