use super::*;

/// The task's explicit case: a `http_download` prompt with no budget
/// available omits the budget line rather than printing a placeholder.
#[test]
fn http_download_without_a_budget_omits_the_budget_line() {
    let tool = session_tool("http_download");
    let arguments = serde_json::json!({"url": "https://example.com/m.bin", "destination": "m.bin"});
    let facts = ApprovalFacts {
        fetch: Some(FetchFacts {
            fetch_body_bytes: 61_440,
            fetch_seconds: 30,
            fetch_redirects: 5,
            download: None,
        }),
        ..ApprovalFacts::default()
    };
    let prompt = approval_prompt(&tool, &arguments, None, &facts, None, None);
    assert!(
        !prompt.contains("remaining download budget"),
        "no budget available, no budget line: {prompt}"
    );
    assert!(
        !prompt.contains("bytes of "),
        "no placeholder arithmetic in place of the wallet: {prompt}"
    );
}

/// A host-command ask states the session grant covers that program with any
/// arguments — a different URL, a different flag set, a POST instead of a
/// GET — not just this argv.
#[test]
fn a_host_command_ask_states_the_grant_covers_any_arguments() {
    let tool = crate::interactive::session_definitions::run_command();
    let facts = host_facts();
    let arguments =
        serde_json::json!({"program": "curl", "args": ["https://example.com/data.csv"]});
    let grant = grant_token(&tool.name, &arguments, None, &facts);
    assert_eq!(grant.as_deref(), Some("command:curl"));
    let prompt = approval_prompt(&tool, &arguments, grant.as_deref(), &facts, None, None);
    assert!(
        prompt.contains("covers curl with any arguments"),
        "the ask names what [s] widens to: {prompt}"
    );
}

/// A fetch ask states the session grant covers that scheme and host with any
/// path — not just this URL.
#[test]
fn a_fetch_ask_states_the_grant_covers_any_path_on_that_host() {
    let facts = session_facts();
    let tool = session_tool("http_fetch");
    let arguments = serde_json::json!({"url": "https://example.com/data.csv"});
    let grant = grant_token(&tool.name, &arguments, None, &facts);
    assert_eq!(grant.as_deref(), Some("fetch:https+example.com"));
    let prompt = approval_prompt(&tool, &arguments, grant.as_deref(), &facts, None, None);
    assert!(
        prompt.contains("covers https on example.com, any path"),
        "the ask names what [s] widens to: {prompt}"
    );
}

/// A workspace-write ask states the session grant covers every workspace
/// write — not this one path.
#[test]
fn a_workspace_write_ask_states_the_grant_covers_every_write() {
    let facts = session_facts();
    let tool = session_tool("workspace_write");
    let arguments = serde_json::json!({"path": "notes/summary.md", "content": "hello world"});
    let grant = grant_token(&tool.name, &arguments, None, &facts);
    assert_eq!(grant.as_deref(), Some("workspace-write"));
    let prompt = approval_prompt(&tool, &arguments, grant.as_deref(), &facts, None, None);
    assert!(
        prompt.contains("covers every workspace write") && prompt.contains("not this path alone"),
        "the ask names what [s] widens to: {prompt}"
    );
}

/// When no session grant is on offer (`grant` is `None`), no scope sentence
/// renders — the ask must never describe a grant the user is not being
/// offered.
#[test]
fn an_ask_with_no_session_grant_offers_no_scope_sentence() {
    let hosts = [
        (
            session_tool("run_program"),
            serde_json::json!({"program": "bench", "args": ["--json"]}),
        ),
        (
            session_tool("http_fetch"),
            serde_json::json!({"url": "https://example.com/data.csv"}),
        ),
        (
            session_tool("workspace_write"),
            serde_json::json!({"path": "notes/summary.md", "content": "hi"}),
        ),
    ];
    let facts = session_facts();
    for (tool, arguments) in hosts {
        let prompt = approval_prompt(&tool, &arguments, None, &facts, None, None);
        for word in ["covers ", "covers every", "any path", "any arguments"] {
            assert!(
                !prompt.contains(word),
                "no grant offered, no scope sentence — {word} in: {prompt}"
            );
        }
    }
    // The host lane shapes its sentences the same way: a run_command call
    // whose composition offers nothing (a path, not a bare name) renders no
    // scope sentence either.
    let tool = crate::interactive::session_definitions::run_command();
    let facts = host_facts();
    let arguments = serde_json::json!({"program": "./evil", "args": []});
    assert_eq!(grant_token(&tool.name, &arguments, None, &facts), None);
    let prompt = approval_prompt(&tool, &arguments, None, &facts, None, None);
    assert!(
        !prompt.contains("covers "),
        "no grant offered, no scope sentence — in: {prompt}"
    );
}

/// The scope sentence is wording only: it does not change what a grant
/// permits — the token produced for a given call is unchanged.
#[test]
fn the_scope_sentence_does_not_change_what_a_grant_permits() {
    let facts = host_facts();
    let session = session_facts();
    let cases = [
        (
            crate::interactive::session_definitions::run_command(),
            serde_json::json!({"program": "curl", "args": ["https://example.com/data.csv"]}),
            Some("command:curl"),
        ),
        (
            session_tool("http_fetch"),
            serde_json::json!({"url": "https://example.com/data.csv"}),
            Some("fetch:https+example.com"),
        ),
        (
            session_tool("workspace_write"),
            serde_json::json!({"path": "notes/summary.md", "content": "hi"}),
            Some("workspace-write"),
        ),
    ];
    for (tool, arguments, expected) in cases {
        let facts = if tool.name == "run_command" {
            &facts
        } else {
            &session
        };
        assert_eq!(
            grant_token(&tool.name, &arguments, None, facts).as_deref(),
            expected,
            "the token for {} is unchanged",
            tool.name
        );
    }
}
