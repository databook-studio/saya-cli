//! Scripted no-database coding through `--workspace --turn-file`: a mock
//! OpenAI-compatible provider answers each round with either a tool call or a
//! final text, and a round may depend on the request body it answers (the
//! digest the model "observed" comes from the read tool's result in that very
//! body). These tests pin, through the real `saya` binary, that a headless
//! one-turn session reads a file, edits it with the digest-guarded tool, and
//! reports — with NO database profile and no `--connections`.

mod grant_surface;
mod harness;

use harness::{
    Round, RoundFn, digest_from, env, last_tool_result, request_json, run, run_background, script,
    sigint, stderr, stdout, tool_names, wait_until_holding,
};
use std::time::Duration;

const FIXTURE: &str = "pub fn add(a: i32, b: i32) -> i32 { a - b }\n";
const EDITED: &str = "pub fn add(a: i32, b: i32) -> i32 { a + b }\n";

/// Every stdout line must parse as JSON (ndjson stays machine-readable), the
/// tool events must arrive requested-then-completed in order, and the stream
/// must end complete.
fn assert_tool_flow(out: &str, expected: &[(&str, &str)]) {
    for line in out.lines().filter(|line| !line.is_empty()) {
        serde_json::from_str::<serde_json::Value>(line).expect("every stdout line is JSON");
    }
    let pairs: Vec<(String, String)> = out
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|event| {
            let kind = event["event"].as_str()?;
            let name = event["name"].as_str()?;
            matches!(kind, "tool_requested" | "tool_completed" | "tool_denied")
                .then(|| (kind.to_owned(), name.to_owned()))
        })
        .collect();
    let expected: Vec<(String, String)> = expected
        .iter()
        .copied()
        .map(|(kind, name)| (kind.to_owned(), name.to_owned()))
        .collect();
    assert_eq!(pairs, expected, "tool events in order; stdout:\n{out}");
    // The stream ends with the one terminal result, and the last event before
    // it is `complete` (the result trails the turn's own stream).
    let lines: Vec<&str> = out.lines().filter(|line| !line.is_empty()).collect();
    assert!(
        lines
            .last()
            .is_some_and(|line| line.contains("\"event\":\"turn_result\"")),
        "the stream ends with the terminal result; stdout:\n{out}"
    );
    assert!(
        lines
            .iter()
            .rev()
            .nth(1)
            .is_some_and(|line| line.contains("\"event\":\"complete\"")),
        "the stream ends complete before the result; stdout:\n{out}"
    );
}

/// The read → guarded-edit → final-text script. Round 2's `expected_digest`
/// is either the `wrong` value (to probe the guard) or the digest parsed out
/// of the read's tool result in the body the round answers — the model must
/// chain exactly what the tool returned.
fn read_edit_rounds(wrong: Option<String>) -> Vec<RoundFn> {
    vec![
        Box::new(|_| Round::Tool {
            name: "workspace_read",
            arguments: serde_json::json!({"path": "src/lib.rs"}),
        }),
        Box::new(move |request| Round::Tool {
            name: "workspace_edit",
            arguments: serde_json::json!({
                "path": "src/lib.rs",
                "old_text": "a - b",
                "new_text": "a + b",
                "expected_digest": wrong.clone().unwrap_or_else(|| digest_from(request)),
            }),
        }),
        Box::new(|_| Round::Text("fixed")),
    ]
}

include!("main/cases/edit.rs");
include!("main/cases/headless.rs");
include!("main/cases/interruption.rs");
include!("main/cases/failure.rs");
