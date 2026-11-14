//! Per-turn reasoning effort.
//!
//! Thinking hard only where the turn needs it keeps routine turns cheap. The
//! policy is decided here, in the loop, immediately before each request; the
//! provider only serializes the effort it is handed.
//!
//! - the first request of a run — there is no prior assistant turn to build on
//!   — thinks hard;
//! - the first request after a *diagnostic* failure thinks hard, so the model
//!   can diagnose rather than repeat the failed call;
//! - a second consecutive diagnostic failure thinks medium: the first
//!   diagnosis did not settle it, but paying high again has not helped;
//! - every other request thinks lightly.
//!
//! A non-zero exit is not by itself a diagnostic surprise. A failing test suite
//! is the task's red state, and exit 1 from `grep`, `rg`, `diff`, `test` or `[`
//! is an ordinary "no" — neither escalates. A test command that never reached
//! its tests (a compile or collection failure) does, as does a second exit code
//! from an ordinary command. An HTTP retry never re-enters this policy: it
//! repeats the identical request.
//!
//! A configured `reasoning_effort` (low/medium/high) pins every request and
//! bypasses the policy entirely. Unset and `adaptive` both mean the policy.

use crate::{ChatMessage, ReasoningEffort, ToolCall};
use std::collections::HashMap;

/// The effort for one request and the failure state the next request sees.
pub(super) struct EffortDecision {
    pub(super) effort: ReasoningEffort,
    /// The trailing results contain a diagnostic failure. A test command's
    /// expected non-zero exit does not count.
    pub(super) diagnostic_failure: bool,
}

pub(super) fn decide(
    messages: &[ChatMessage],
    first_request: bool,
    previous_diagnostic_failure: bool,
    configured: Option<ReasoningEffort>,
) -> EffortDecision {
    let diagnostic_failure = trailing_diagnostic_failure(messages);
    let effort = match configured {
        Some(fixed) => fixed,
        None if first_request => ReasoningEffort::High,
        None if diagnostic_failure && previous_diagnostic_failure => ReasoningEffort::Medium,
        None if diagnostic_failure => ReasoningEffort::High,
        None => ReasoningEffort::Low,
    };
    EffortDecision {
        effort,
        diagnostic_failure,
    }
}

/// The tool results immediately preceding this request. A continuation note or
/// a user message ends the run of results: the failure it may describe is
/// already in the note's own text, not in a result the model can act on.
fn trailing_diagnostic_failure(messages: &[ChatMessage]) -> bool {
    let calls = tool_calls_by_id(messages);
    messages
        .iter()
        .rev()
        .take_while(|message| message.role == "tool")
        .any(|message| result_is_diagnostic_failure(message, &calls))
}

/// Every tool call still in the conversation, keyed by the id its result
/// carries. A result whose call is no longer present is classified on its own
/// fields alone — the conservative reading.
fn tool_calls_by_id(messages: &[ChatMessage]) -> HashMap<&str, &ToolCall> {
    messages
        .iter()
        .flat_map(|message| message.tool_calls.iter())
        .map(|call| (call.id.as_str(), call))
        .collect()
}

fn result_is_diagnostic_failure(message: &ChatMessage, calls: &HashMap<&str, &ToolCall>) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&message.content) else {
        // A truncated or redacted result that no longer parses says nothing
        // reliable about success; the model still sees the text.
        return false;
    };
    // `"error": null` is the absence of an error, not a failure.
    if value.get("error").is_some_and(|error| !error.is_null()) {
        return true;
    }
    if value.get("timed_out").and_then(serde_json::Value::as_bool) == Some(true) {
        return true;
    }
    if value
        .pointer("/post_edit_diagnostics/diagnostics")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|diagnostics| !diagnostics.is_empty())
    {
        return true;
    }
    let Some(exit_code) = value.get("exit_code").and_then(serde_json::Value::as_i64) else {
        return false;
    };
    if exit_code == 0 {
        return false;
    }
    let command = message
        .tool_call_id
        .as_deref()
        .and_then(|id| calls.get(id))
        .filter(|call| is_shell(&call.name))
        .and_then(|call| call.arguments.get("command"))
        .and_then(serde_json::Value::as_str);
    // A non-zero exit with no command to read is the conservative reading.
    let Some(command) = command else {
        return true;
    };
    if is_test_command(command) {
        return test_failure_is_diagnostic(command, exit_code, &value);
    }
    // Exit 1 of grep/rg/diff/test/[ means "the answer is no", not "an error".
    !(exit_code == 1 && is_ordinary_nonzero_command(command))
}

/// A test command that never reached its tests is diagnostic. A failing suite
/// is not: cargo reports both with a non-zero exit, so the compile marker, not
/// the code, separates them; pytest documents 2–5 as collection/usage/internal
/// stops.
fn test_failure_is_diagnostic(command: &str, exit_code: i64, value: &serde_json::Value) -> bool {
    if is_cargo_test(command) {
        return ["stdout", "stderr"].iter().any(|field| {
            value
                .get(*field)
                .and_then(serde_json::Value::as_str)
                .is_some_and(|text| text.contains("error[E") || text.contains("could not compile"))
        });
    }
    if is_pytest(command) {
        return matches!(exit_code, 2..=5);
    }
    false
}

fn is_shell(name: &str) -> bool {
    matches!(name, "shell" | "run_command" | "bash")
}

/// Whether a shell command is only a test run. A mixed command (`cargo build
/// && cargo test`) is not: the non-zero exit may be the build's, and the model
/// cannot tell which segment failed from the result alone.
fn is_test_command(command: &str) -> bool {
    every_segment(command, is_test_invocation)
}

/// Whether every meaningful segment is one whose exit 1 is an ordinary "no".
fn is_ordinary_nonzero_command(command: &str) -> bool {
    every_segment(command, is_ordinary_invocation)
}

fn is_cargo_test(command: &str) -> bool {
    every_segment(command, |words| {
        matches!(words, ["cargo", "test", ..] | ["cargo", "nextest", ..])
    })
}

fn is_pytest(command: &str) -> bool {
    every_segment(command, |words| {
        matches!(
            words,
            ["pytest", ..] | ["py.test", ..] | ["python" | "python3", "-m", "pytest", ..]
        )
    })
}

/// Whether every meaningful segment of the command satisfies `kind`, and at
/// least one does. A wrapper-only command (`cd repo`) matches nothing.
fn every_segment(command: &str, kind: impl Fn(&[&str]) -> bool) -> bool {
    let mut saw = false;
    for segment in command.split(['\n', ';']) {
        for part in segment.split("&&").flat_map(|part| part.split("||")) {
            // Output filters after a pipe do not change which command failed.
            let words: Vec<&str> = part
                .split('|')
                .next()
                .unwrap_or("")
                .split_whitespace()
                .collect();
            let words = strip_wrappers(&words);
            if words.is_empty() || is_ignorable(words) {
                continue;
            }
            if !kind(words) {
                return false;
            }
            saw = true;
        }
    }
    saw
}

/// Drop leading environment assignments and harmless wrappers (`cd repo`,
/// `timeout 300`, `env FOO=1`) so the command itself is the first words.
fn strip_wrappers<'a>(mut words: &'a [&'a str]) -> &'a [&'a str] {
    loop {
        match words {
            ["cd", _, rest @ ..] | ["timeout", _, rest @ ..] | ["env", rest @ ..] => words = rest,
            [assignment, rest @ ..] if is_assignment(assignment) => words = rest,
            _ => return words,
        }
    }
}

fn is_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn is_ignorable(words: &[&str]) -> bool {
    matches!(
        words.first().copied(),
        Some("cd" | "echo" | "printf" | "set" | "export" | "true" | ":" | "pwd" | "sleep")
    )
}

/// Commands whose exit 1 means "the answer is no": no match, a difference, or
/// a false condition. Their exit 2 and above is a real error.
fn is_ordinary_invocation(words: &[&str]) -> bool {
    matches!(
        words,
        ["grep" | "egrep" | "fgrep" | "rg" | "ag", ..]
            | ["diff", ..]
            | ["test", ..]
            | ["[", ..]
            | ["git", "grep" | "diff", ..]
    )
}

fn is_test_invocation(words: &[&str]) -> bool {
    matches!(
        words,
        ["cargo", "test", ..]
            | ["cargo", "nextest", ..]
            | ["pytest", ..]
            | ["py.test", ..]
            | ["python" | "python3", "-m", "pytest", ..]
            | ["npm", "test", ..]
            | ["npm", "run", "test", ..]
            | ["yarn", "test", ..]
            | ["pnpm", "test", ..]
            | ["bun", "test", ..]
            | ["npx", "jest" | "vitest", ..]
            | [
                "jest" | "vitest" | "mocha" | "rspec" | "phpunit" | "ctest",
                ..
            ]
            | ["go", "test", ..]
            | ["make", "test", ..]
            | ["mvn" | "./mvnw", "test", ..]
            | ["gradle" | "./gradlew", "test", ..]
            | ["dotnet", "test", ..]
            | ["mix", "test", ..]
            | ["swift", "test", ..]
    )
}

#[cfg(test)]
#[path = "reasoning_policy_tests.rs"]
mod tests;
