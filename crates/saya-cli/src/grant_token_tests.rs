//! The grant token suggestion: the bridge from one tool call to the
//! narrowest `--allow` grammar token a session grant for it would record.
//! Every token the suggester can produce must be a word the existing
//! grammar parser accepts — the parser is the authority, never a duplicate
//! of it — and a call the slice cannot name must yield `None`, which means
//! the tool keeps asking every call.

use crate::agent::tools::DatabaseTools;
use crate::commands::run::scopes;
use crate::connection::ConnectionRegistry;
use crate::grant_token::{SQL_FAMILY, grant_token, session_answers_line};
use async_trait::async_trait;
use saya_agent::{ApprovalDecision, ApprovalPolicy, LocalStateEffect, SessionPolicy};
use saya_connectors::DatabaseConnector;
use saya_harness::fetch::FetchDestination;
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};
use serde_json::{Value, json};

/// The token a call suggests, for the tests that need it as a value. The
/// suggester reads the turn's primary through the third argument; `None`
/// here means no primary is bound.
fn token_for(tool: &str, arguments: Value) -> String {
    grant_token(tool, &arguments, None)
        .unwrap_or_else(|| panic!("{tool} with {arguments} must suggest a token for this test"))
}

/// The suggester's answer for one SQL-family call shape.
fn sql_suggestion(tool: &str, arguments: Value, registry: &ConnectionRegistry) -> Option<String> {
    grant_token(tool, &arguments, registry.primary())
}

/// True when the parsed approval actually contains what `token` names —
/// the parse alone is not enough, the capability must be the named one.
/// A `sql:` token names no run capability: on the session surface it is
/// carried as the grant store's word, so the round-trip is the verbatim
/// token.
fn approves_what_it_names(token: &str, approved: &scopes::Approved) -> bool {
    if token.starts_with("sql:") {
        return approved.tokens.iter().any(|stated| stated == token);
    }
    let capabilities = &approved.capabilities;
    match token {
        "workspace-write" => capabilities.workspace_write,
        "scratch" => capabilities.scratch,
        other => match other.split_once(':') {
            Some(("fetch", rest)) => capabilities.fetch.as_ref().is_some_and(|scope| {
                scope
                    .destinations
                    .iter()
                    .any(|d| format!("{}+{}", d.scheme, d.host) == rest)
            }),
            Some(("runner", program)) => capabilities
                .runner
                .as_ref()
                .is_some_and(|scope| scope.programs.iter().any(|allowed| allowed == program)),
            Some(("interpreter", program)) => capabilities
                .interpreter
                .as_ref()
                .is_some_and(|scope| scope.programs.iter().any(|allowed| allowed == program)),
            _ => false,
        },
    }
}

struct DummyConnector {
    dialect: SqlDialect,
}

#[async_trait]
impl DatabaseConnector for DummyConnector {
    fn dialect(&self) -> SqlDialect {
        self.dialect
    }

    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }

    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Err(ConnectionError::schema_failed("dummy"))
    }

    async fn execute(&self, _: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Err(ConnectionError::query_failed("dummy"))
    }
}

/// A registry whose primary named `name` is connected — the shape the turn
/// builder produces when a profile is selected. Shared with the approval
/// tests, which bind it into a decider's primary handle.
pub(crate) fn registry_with_primary(name: &str) -> ConnectionRegistry {
    let mut registry = ConnectionRegistry::new(name);
    registry.insert(
        name,
        crate::connection::ConnectionEntry {
            connector: Box::new(DummyConnector {
                dialect: SqlDialect::Postgres,
            }),
            dialect: SqlDialect::Postgres,
            profile_id: None,
        },
    );
    registry
}

/// A registry with no connection — the shape the turn builder produces when
/// no profile is selected (`ConnectionRegistry::new("")`, nothing inserted).
fn empty_registry() -> ConnectionRegistry {
    ConnectionRegistry::new("")
}

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
        let token = token_for(&tool, arguments);
        let approved = scopes::parse(std::slice::from_ref(&token), scopes::Surface::Session)
            .unwrap_or_else(|error| panic!("`{token}` must parse under /allow: {error}"));
        assert!(
            approves_what_it_names(&token, &approved),
            "`{token}` must approve exactly the capability it names, got: {:?}",
            approved.capabilities
        );
    }
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
    let definitions = DatabaseTools::definitions(true, false, false, false);
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
    let definitions = DatabaseTools::definitions(true, false, false, false);
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
            registry.primary()
        ),
        None,
        "the fan-out's referent can grow after approval — it suggests no token"
    );
    assert_eq!(
        grant_token(
            "render_chart",
            &json!({"sql": "SELECT 1", "chart_type": "bar", "connection": "analytics"}),
            registry.primary()
        ),
        None,
        "render_chart writes a file and opens a browser — not a `sql:` grant's \
         words"
    );
}

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
    let closed = DatabaseTools::definitions(false, false, false, false);
    let open = DatabaseTools::definitions(true, false, false, false);
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
            grant_token(tool, &json!({"sql": "SELECT 1"}), registry.primary()),
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
        let token = token_for("http_fetch", json!({"url": url}));
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

/// A seeded token and a suggested token are the same string for the same
/// destination: `/allow` parses through the grammar's parser, which
/// normalises the fetch family to the URL parser's spelling — the spelling
/// the suggester produces from the call's URL. Whatever casing the user
/// types, the grant pre-answers the call it names (U6 defect 2: the verbatim
/// seed never matched the lowercased suggestion, so `/grants` listed a
/// grant that pre-answered nothing).
#[test]
fn a_seeded_token_and_a_suggested_token_are_the_same_string_for_a_destination() {
    let suggested = token_for("http_fetch", json!({"url": "https://Example.com/x"}));
    let approved = scopes::parse(
        &["fetch:HTTPS+Example.com".to_string()],
        scopes::Surface::Session,
    )
    .expect("a mixed-case fetch token parses on the session surface");
    assert_eq!(
        approved.tokens,
        vec![suggested],
        "the seeded token and the suggested token are one string for one destination"
    );
}

/// A malformed or absent argument yields `None`, never a guessed token —
/// and `None` means the tool keeps asking every call.
#[test]
fn a_malformed_or_absent_argument_yields_none_never_a_token() {
    assert_eq!(grant_token("http_fetch", &json!({}), None), None, "no url");
    assert_eq!(
        grant_token("http_fetch", &json!({"url": "not a url"}), None),
        None,
        "not a URL"
    );
    assert_eq!(
        grant_token(
            "http_fetch",
            &json!({"url": "mailto:someone@example.com"}),
            None
        ),
        None,
        "a scheme with no host"
    );
    assert_eq!(
        grant_token("http_fetch", &json!({"url": ""}), None),
        None,
        "empty url"
    );
    assert_eq!(
        grant_token("http_fetch", &json!({"url": 7}), None),
        None,
        "non-string url"
    );
    assert_eq!(
        grant_token("http_download", &json!({"destination": "f.bin"}), None),
        None,
        "download without url"
    );
    assert_eq!(
        grant_token("run_program", &json!({}), None),
        None,
        "no program"
    );
    assert_eq!(
        grant_token("run_program", &json!({"program": ""}), None),
        None,
        "empty program"
    );
    assert_eq!(
        grant_token("run_program", &json!({"program": "/usr/bin/env"}), None),
        None,
        "paths are never programs"
    );
    assert_eq!(
        grant_token("run_program", &json!({"program": ".."}), None),
        None,
        "traversal is never a program"
    );
    assert_eq!(
        grant_token("run_program", &json!({"program": 7}), None),
        None,
        "non-string program"
    );
}

/// The interpreter/runner split is the run engine's own rule (the grammar
/// mirror at parse time), not an invented one: a name the runner refuses is
/// the interpreter family's member, everything else rides the runner family.
#[test]
fn run_program_s_family_rule_is_the_run_engine_s() {
    for (program, family) in [
        ("python3", "interpreter"),
        ("python", "interpreter"),
        ("bash", "interpreter"),
        ("sh", "interpreter"),
        ("node", "interpreter"),
        ("ripgrep", "runner"),
        ("ls", "runner"),
        ("git", "runner"),
        ("script", "interpreter"),
    ] {
        assert_eq!(
            token_for("run_program", json!({"program": program})),
            format!("{family}:{program}"),
            "the family must match the run engine's refusal list"
        );
    }
}

/// The prompt's answers line names the grantable token only when one exists;
/// with none it offers the two answers and says why the third is absent.
#[test]
fn the_answers_line_names_the_token_only_when_one_exists() {
    assert_eq!(
        session_answers_line(Some("workspace-write")),
        "[a] allow once   [s] allow workspace-write for this session   [d] deny",
        "the token is named verbatim — the word the grant records"
    );
    let without = session_answers_line(None);
    assert_eq!(
        without, "[a] allow once   [d] deny   (no session grant for this tool)",
        "with no token the line offers two answers and says so"
    );
}
