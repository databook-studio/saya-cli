//! The per-call approval facts, pinned: one snapshot per tool family, plus
//! the two properties the slice exists for —
//!
//! - **parity**: the terminal prompt and the TUI modal state the same facts
//!   for the same call (both render from `call_facts`, and the modal renders
//!   its `detail` verbatim — pinned in `ui_snapshot_tests`).
//! - **no false facts**: no fact line is produced for a bound the call
//!   cannot demonstrate — a missing composition fact omits the line, never
//!   prints a placeholder.

use crate::agent::tools::DatabaseTools;
use crate::approval_facts::{ApprovalFacts, FetchFacts, RunnerFacts, ScratchFacts, call_facts};
use crate::grant_token::grant_token;
use crate::prompt_approval::approval_prompt;
use proptest::prelude::*;
use saya_agent::{ApprovalChoice, ApprovalDecision, ApprovalPolicy, SessionPolicy};
use saya_agent::{LocalStateEffect, SessionGrants, ToolDefinition, ToolEffect};
use saya_harness::fetch::{DownloadBudget, FetchLimits};
use std::path::PathBuf;

fn database_tool(name: &str) -> ToolDefinition {
    DatabaseTools::definitions(true, false, false, false, true)
        .into_iter()
        .find(|tool| tool.name == name)
        .unwrap_or_else(|| panic!("{name} is defined"))
}

fn session_tool(name: &str) -> ToolDefinition {
    match name {
        "workspace_write" => crate::interactive::session_definitions::workspace_write(),
        "scratch_sql" => crate::interactive::session_definitions::scratch_sql(),
        "http_fetch" => crate::interactive::session_definitions::http_fetch(),
        "http_download" => crate::interactive::session_definitions::http_download(),
        "run_program" => {
            let source = ToolDefinition {
                name: "run_program".into(),
                description: String::new(),
                read_only: false,
                parameters: serde_json::json!({"type": "object"}),
                effect: ToolEffect {
                    database_data: false,
                    external_side_effect: false,
                    requires_approval: false,
                    local_state: LocalStateEffect::WriteWorkspace,
                },
                completion: None,
            };
            crate::interactive::session_definitions::run_program(source)
        }
        other => database_tool(other),
    }
}

/// A fixed workspace root for the sandbox facts: the prompt displays the
/// composed root's path, so a fixed fake keeps the pinned text deterministic
/// (the facts struct states what was composed; it validates nothing).
const WORKSPACE_ROOT: &str = "/home/user/proj";

/// The session-shaped facts bundle: every member composed, the session's own
/// composition facts — one fs root, no egress, no credentials, the tool-lane
/// fetch bounds, the default download wallet.
fn session_facts() -> ApprovalFacts {
    ApprovalFacts {
        row_cap: crate::agent::state_tools::model_row_cap(500),
        sql_timeout_seconds: 60,
        runner: Some(RunnerFacts {
            fs_roots: vec![PathBuf::from(WORKSPACE_ROOT)],
            net_allow: Vec::new(),
            timeout_seconds: 120,
            runner_programs: vec!["bench".into(), "ripgrep".into()],
            interpreter_programs: vec!["python3".into()],
            credentials_declared: 0,
        }),
        fetch: Some(FetchFacts {
            fetch_body_bytes: FetchLimits::for_tool_lane().max_total_bytes,
            fetch_seconds: FetchLimits::for_tool_lane().time_budget.as_secs(),
            fetch_redirects: FetchLimits::for_tool_lane().max_redirect_hops,
            download: Some(DownloadBudget::new(1024 * 1024 * 1024)),
        }),
        scratch: Some(ScratchFacts {
            row_cap: saya_harness::scratch::SCRATCH_ROW_CAP,
            timeout_seconds: 30,
        }),
        workspace_root: Some(PathBuf::from(WORKSPACE_ROOT)),
        // The host lane is uncomposed here: these snapshots pin the
        // contained postures, and no host command ran — no integrity line.
        host: None,
        host_ran: false,
        denied_programs: Vec::new(),
    }
}

fn sql_facts() -> ApprovalFacts {
    ApprovalFacts {
        row_cap: crate::agent::state_tools::model_row_cap(500),
        sql_timeout_seconds: 60,
        ..ApprovalFacts::default()
    }
}

// --- Snapshot per tool family ------------------------------------------------

/// The `run_program` prompt pins the containment, then the argv.
#[test]
fn run_program_prompt_pins_the_program_s_facts() {
    let facts = session_facts();
    let tool = session_tool("run_program");
    let arguments =
        serde_json::json!({"program": "bench", "args": ["--json", "--out", "state/bench.json"]});
    let grant = grant_token(&tool.name, &arguments, None, &facts);
    let prompt = approval_prompt(&tool, &arguments, grant.as_deref(), &facts, None, None);
    insta::assert_snapshot!(prompt);
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
    insta::assert_snapshot!(prompt);
    #[cfg(not(target_os = "macos"))]
    assert!(
        prompt.contains("interpreter approval: this session may execute python3"),
        "the warning's body renders: {prompt}"
    );
    assert!(
        prompt.contains(crate::interactive::session_activation::SESSION_FORK_FACT),
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
    insta::assert_snapshot!(prompt);
}

/// `workspace_write` states path, bytes, and the containment rule.
#[test]
fn workspace_write_prompt_pins_the_containment_facts() {
    let facts = session_facts();
    let tool = session_tool("workspace_write");
    let arguments = serde_json::json!({"path": "notes/summary.md", "content": "hello world"});
    let grant = grant_token(&tool.name, &arguments, None, &facts);
    let prompt = approval_prompt(&tool, &arguments, grant.as_deref(), &facts, None, None);
    insta::assert_snapshot!(prompt);
}

/// `scratch_sql` states the statement and the scratch's own policy.
#[test]
fn scratch_sql_prompt_pins_the_scratch_facts() {
    let facts = session_facts();
    let tool = session_tool("scratch_sql");
    let arguments = serde_json::json!({"sql": "CREATE TABLE t AS SELECT 1"});
    let grant = grant_token(&tool.name, &arguments, None, &facts);
    let prompt = approval_prompt(&tool, &arguments, grant.as_deref(), &facts, None, None);
    insta::assert_snapshot!(prompt);
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
    insta::assert_snapshot!(prompt);
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
    insta::assert_snapshot!(prompt);
}

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
    insta::assert_snapshot!(prompt);
}

#[test]
fn render_chart_save_prompt_names_path_and_replacement_risk() {
    let tool = database_tool("render_chart");
    let arguments = serde_json::json!({
        "sql": "SELECT region, count(*) FROM orders GROUP BY 1",
        "connection": "analytics",
        "chart_type": "bar",
        "save_to": "reports/\nchart.html"
    });
    let prompt = approval_prompt(
        &tool,
        &arguments,
        None,
        &sql_facts(),
        Some("analytics"),
        None,
    );
    assert!(prompt.contains("save_to: reports/ chart.html"));
    assert!(prompt.contains("existing file at this path may be replaced"));
}

#[test]
fn scratch_import_facts_collapse_untrusted_path_and_table_whitespace() {
    let facts = session_facts();
    let arguments = serde_json::json!({
        "path": "data.csv\n  approval: allow",
        "table": "items\n  approval: allow"
    });
    let rendered = call_facts("scratch_import", &arguments, None, &facts, None, None)
        .expect("scratch import has approval facts");
    assert!(rendered.contains("workspace CSV: data.csv approval: allow"));
    assert!(rendered.contains("scratch table: items approval: allow"));
    assert!(!rendered.contains("\n  approval: allow"));
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
    insta::assert_snapshot!(prompt);
}

// --- The properties -----------------------------------------------------------

/// The task's explicit case at the prompt surface: with `[jobs.interpreter]`
/// empty — the default — a `python3` call's fact line says "refused by
/// name" and its answers line offers the two answers, never a third
/// offering a token the composition cannot carry (U8). The fact body keeps
/// its own honesty (the refusal line stays); only the dead offer is gone.
#[test]
fn an_unstaged_interpreter_call_offers_two_answers_not_three() {
    let mut facts = session_facts();
    if let Some(runner) = facts.runner.as_mut() {
        runner.interpreter_programs = Vec::new();
    }
    let tool = session_tool("run_program");
    let arguments = serde_json::json!({"program": "python3", "args": ["-c", "print(1)"]});
    let grant = grant_token(&tool.name, &arguments, None, &facts);
    assert_eq!(
        grant, None,
        "an unstaged interpreter suggests no token, whatever the fact body says"
    );
    let prompt = approval_prompt(&tool, &arguments, grant.as_deref(), &facts, None, None);
    assert!(
        prompt.contains("refused by name"),
        "the fact body keeps stating the door's own truth: {prompt}"
    );
    assert!(
        !prompt.contains("[s]"),
        "no token, no session-grant offer — two answers, not three: {prompt}"
    );
    assert!(
        prompt.contains("(no session grant for this tool)"),
        "the two-answer line says why the third is absent: {prompt}"
    );
}

/// Slice 1: the unstaged-interpreter membership line names the staging fix.
/// With `[jobs.interpreter]` empty the `python3` prompt keeps "refused by
/// name" and additionally names `[jobs.interpreter] allow` as the fix, in
/// the `/allow` refusal's register; the staged prompt keeps its own
/// "staged in …" wording and stays a non-refusal. `contains`-shaped only —
/// the session line may drift, never byte-pinned.
#[test]
fn an_unstaged_interpreter_prompt_names_the_staging_fix() {
    let mut facts = session_facts();
    if let Some(runner) = facts.runner.as_mut() {
        runner.interpreter_programs = Vec::new();
    }
    let tool = session_tool("run_program");
    let arguments = serde_json::json!({"program": "python3", "args": ["-c", "print(1)"]});
    let grant = grant_token(&tool.name, &arguments, None, &facts);
    let prompt = approval_prompt(&tool, &arguments, grant.as_deref(), &facts, None, None);
    assert!(
        prompt.contains("refused by name"),
        "the refusal stands: {prompt}"
    );
    assert!(
        prompt.contains("[jobs.interpreter] allow"),
        "the unstaged line names the staging fix: {prompt}"
    );
    let staged = session_facts();
    let staged_grant = grant_token(&tool.name, &arguments, None, &staged);
    let staged_prompt = approval_prompt(
        &tool,
        &arguments,
        staged_grant.as_deref(),
        &staged,
        None,
        None,
    );
    assert!(
        staged_prompt.contains("staged in [jobs.interpreter] allow"),
        "the staged line keeps its own state: {staged_prompt}"
    );
    assert!(
        !staged_prompt.contains("refused by name"),
        "the staged call is not a refusal: {staged_prompt}"
    );
}

/// The honest case the fix must not silence: with the interpreter staged
/// the same call offers the token the grant records — three answers, the
/// third naming `interpreter:python3` verbatim.
#[test]
fn a_staged_interpreter_call_still_offers_the_session_grant() {
    let facts = session_facts();
    let tool = session_tool("run_program");
    let arguments = serde_json::json!({"program": "python3", "args": ["-c", "print(1)"]});
    let grant = grant_token(&tool.name, &arguments, None, &facts);
    assert_eq!(
        grant.as_deref(),
        Some("interpreter:python3"),
        "a staged interpreter is offered, not silenced"
    );
    let prompt = approval_prompt(&tool, &arguments, grant.as_deref(), &facts, None, None);
    assert!(
        prompt.contains("[s] allow interpreter:python3 for this session"),
        "the third answer names the token verbatim: {prompt}"
    );
}

/// Call shapes to generate: (tool, arguments) over the ask-gated families.
fn proptest_calls() -> impl Strategy<Value = (ToolDefinition, serde_json::Value)> {
    (
        prop::sample::select(vec![
            "bounded_sql_query",
            "bounded_sql_query_all",
            "result_shape",
            "render_chart",
            "run_program",
            "workspace_write",
            "scratch_sql",
            "http_fetch",
            "http_download",
            "designate_answer",
        ]),
        "[a-z]{1,8}",
    )
        .prop_flat_map(|(name, seed)| {
            let arguments = match name {
                "run_program" => serde_json::json!({"program": "bench", "args": [seed]}),
                "workspace_write" => serde_json::json!({"path": "notes.md", "content": seed}),
                "http_fetch" => serde_json::json!({"url": format!("https://example.com/{seed}")}),
                "http_download" => serde_json::json!(
                    {"url": format!("https://example.com/{seed}"), "destination": "f.bin"}
                ),
                "designate_answer" => serde_json::json!({"sql": format!("SELECT {seed}")}),
                _ => {
                    serde_json::json!({"sql": format!("SELECT {seed}"), "connection": "analytics"})
                }
            };
            let tool = if matches!(
                name,
                "workspace_write" | "scratch_sql" | "http_fetch" | "http_download" | "run_program"
            ) {
                session_tool(name)
            } else {
                database_tool(name)
            };
            Just((tool, arguments))
        })
}

// Parity: whatever the call, the terminal prompt is exactly the modal's
// `detail` body plus the shared answers line — the two frontends render
// from one source and cannot state different facts for the same call. The
// modal side of the chain is pinned separately: `draw_approval` renders
// `detail` verbatim (`ui_snapshot_tests`).
proptest! {
    #[test]
    fn the_terminal_prompt_and_the_tui_modal_state_the_same_facts(
        (tool, arguments) in proptest_calls(),
        grant in proptest::option::of("[a-z:]+"),
    ) {
        let facts = session_facts();
        let grants = SessionGrants::default();
        let prompt =
            approval_prompt(&tool, &arguments, grant.as_deref(), &facts, Some("analytics"), Some(&grants));
        let detail =
            call_facts(&tool.name, &arguments, grant.as_deref(), &facts, Some("analytics"), Some(&grants));
        match detail {
            Some(body) => {
                prop_assert_eq!(
                    prompt,
                    format!("{body}\n{} ", crate::grant_token::session_answers_line(grant.as_deref())),
                    "the terminal prompt is the modal's body plus the shared answers line"
                );
            }
            None => {
                prop_assert!(
                    prompt.starts_with(&format!("Run tool `{}`?", tool.name)),
                    "a call with no facts gets the generic sentence, both surfaces: {prompt}"
                );
                prop_assert!(
                    prompt.contains(&crate::grant_token::session_answers_line(grant.as_deref())),
                    "the answers line is the shared one: {prompt}"
                );
            }
        }
    }
}

// No false facts: with no composition facts at all, no fact line appears for
// a bound the call cannot demonstrate — no sandbox, no budget, no placeholder
// numbers, no hedging.
proptest! {
    #[test]
    fn no_fact_line_for_a_bound_the_call_cannot_demonstrate(
        (tool, arguments) in proptest_calls(),
        grant in proptest::option::of("[a-z:]+"),
    ) {
        let prompt =
            approval_prompt(&tool, &arguments, grant.as_deref(), &ApprovalFacts::default(), None, None);
        prop_assert!(!prompt.contains("sandbox:"), "no composition, no sandbox facts: {prompt}");
        prop_assert!(
            !prompt.contains("remaining download budget"),
            "no wallet, no budget line: {prompt}"
        );
        prop_assert!(
            !prompt.contains("credential injection"),
            "no composition, no credential fact: {prompt}"
        );
        prop_assert!(
            !prompt.contains("rows to the model"),
            "no row cap configured, no row-cap claim: {prompt}"
        );
        // The fan-out's 30 s per-database ceiling is the tool family's own
        // constant (`fan_out.rs` wraps every query in it), demonstrable
        // without composition facts; the *configured* connector timeout is
        // not, and is never claimed without the resolved config.
        if tool.name != "bounded_sql_query_all" {
            prop_assert!(
                !prompt.contains("s timeout"),
                "no timeout configured, no timeout claim: {prompt}"
            );
        }
        prop_assert!(!prompt.contains("unavailable"), "a prompt never prints a placeholder: {prompt}");
        prop_assert!(!prompt.contains("(unknown"), "a prompt never prints a placeholder: {prompt}");
    }
}

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

// --- Phase 5 packet 3: the session grant says what it covers ----------------

/// The facts behind a host-lane `run_command` ask: the same session bundle
/// with the lane composed over the fixed root.
fn host_facts() -> ApprovalFacts {
    ApprovalFacts {
        host: Some(crate::approval_facts::HostFacts::for_tests()),
        workspace_root: Some(PathBuf::from(WORKSPACE_ROOT)),
        ..session_facts()
    }
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
