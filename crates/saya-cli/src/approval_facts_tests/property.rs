use super::*;
use proptest::prelude::*;

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
            "workspace_edit",
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
                "workspace_edit" => serde_json::json!({
                    "path": format!("notes/{seed}.md"),
                    "old_text": format!("old-{seed}"),
                    "new_text": format!("new-{seed}"),
                    "expected_digest": "sha256:0",
                }),
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
                "workspace_write"
                    | "workspace_edit"
                    | "scratch_sql"
                    | "http_fetch"
                    | "http_download"
                    | "run_program"
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
