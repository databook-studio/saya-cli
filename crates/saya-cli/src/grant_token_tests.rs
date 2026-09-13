//! The grant token suggestion: the bridge from one tool call to the
//! narrowest `--allow` grammar token a session grant for it would record.
//! Every token the suggester can produce must be a word the existing
//! grammar parser accepts — the parser is the authority, never a duplicate
//! of it — and a call the slice cannot name must yield `None`, which means
//! the tool keeps asking every call.

use crate::commands::run::scopes;
use crate::grant_token::{grant_token, session_answers_line};
use saya_harness::fetch::FetchDestination;
use serde_json::json;

/// The token a call suggests, for the tests that need it as a value.
fn token_for(tool: &str, arguments: serde_json::Value) -> String {
    grant_token(tool, &arguments)
        .unwrap_or_else(|| panic!("{tool} with {arguments} must suggest a token for this test"))
}

/// True when the parsed capabilities actually contain what `token` names —
/// the parse alone is not enough, the capability must be the named one.
fn approves_what_it_names(token: &str, capabilities: &saya_types::Capabilities) -> bool {
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

/// The hard rule: every token the suggester produces parses under the
/// `--allow` grammar *and* approves the capability it names. Each case here
/// is a shape the suggester can produce; the parser (widened to `pub(crate)`)
/// is the authority it is judged against.
#[test]
fn every_suggestible_token_parses_to_the_capability_it_names() {
    let cases: Vec<(String, serde_json::Value)> = vec![
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
    ];
    for (tool, arguments) in cases {
        let token = token_for(&tool, arguments);
        let approved = scopes::parse(std::slice::from_ref(&token))
            .unwrap_or_else(|error| panic!("`{token}` must parse under --allow: {error}"));
        assert!(
            approves_what_it_names(&token, &approved.capabilities),
            "`{token}` must approve exactly the capability it names, got: {:?}",
            approved.capabilities
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

/// A malformed or absent argument yields `None`, never a guessed token —
/// and `None` means the tool keeps asking every call.
#[test]
fn a_malformed_or_absent_argument_yields_none_never_a_token() {
    assert_eq!(grant_token("http_fetch", &json!({})), None, "no url");
    assert_eq!(
        grant_token("http_fetch", &json!({"url": "not a url"})),
        None,
        "not a URL"
    );
    assert_eq!(
        grant_token("http_fetch", &json!({"url": "mailto:someone@example.com"})),
        None,
        "a scheme with no host"
    );
    assert_eq!(
        grant_token("http_fetch", &json!({"url": ""})),
        None,
        "empty url"
    );
    assert_eq!(
        grant_token("http_fetch", &json!({"url": 7})),
        None,
        "non-string url"
    );
    assert_eq!(
        grant_token("http_download", &json!({"destination": "f.bin"})),
        None,
        "download without url"
    );
    assert_eq!(grant_token("run_program", &json!({})), None, "no program");
    assert_eq!(
        grant_token("run_program", &json!({"program": ""})),
        None,
        "empty program"
    );
    assert_eq!(
        grant_token("run_program", &json!({"program": "/usr/bin/env"})),
        None,
        "paths are never programs"
    );
    assert_eq!(
        grant_token("run_program", &json!({"program": ".."})),
        None,
        "traversal is never a program"
    );
    assert_eq!(
        grant_token("run_program", &json!({"program": 7})),
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

/// The tools this slice does not grant — every SQL tool included, by
/// deliberate design — get `None`, which means they keep asking every call.
#[test]
fn the_sql_tools_and_everything_else_get_no_token() {
    for tool in [
        "bounded_sql_query",
        "bounded_sql_query_all",
        "schema_discovery",
        "workspace_read",
        "render_chart",
        "result_shape",
        "column_health",
        "join_check",
        "designate_answer",
        "no_such_tool",
    ] {
        assert_eq!(
            grant_token(tool, &json!({"sql": "SELECT 1"})),
            None,
            "{tool} must keep asking every call — this slice grants no SQL tool"
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
