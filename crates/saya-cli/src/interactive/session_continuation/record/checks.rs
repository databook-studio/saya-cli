//! The check half of the collector: which `run_command` calls ended, with
//! what outcome, labeled from the request's argv — redacted, one line,
//! bounded — or the summary's program alone when no request matches.

use super::CheckObservation;
use saya_agent::AgentEvent;
use saya_types::{CheckOutcome, MAX_CHECK_FACTS, MAX_CHECK_LABEL_CHARS, redact};

/// Records the checks the events attest: a `run_command` completion whose
/// summary parses to an exit code. Requests and completions pair by order
/// and name within the same output — every completion or denial consumes
/// the request ahead of it, so a failed or denied call cannot mislabel the
/// next one. `run_program`, unparsable summaries, and denied calls record
/// nothing.
pub(super) fn absorb_checks(
    checks: &mut Vec<CheckObservation>,
    events: &[AgentEvent],
    now_unix_ms: i64,
) {
    let mut requested: Vec<&serde_json::Value> = Vec::new();
    for event in events {
        match event {
            AgentEvent::ToolRequested {
                name, arguments, ..
            } if name == "run_command" => requested.push(arguments),
            AgentEvent::ToolCompleted { name, summary } if name == "run_command" => {
                let arguments = pending(&mut requested);
                if let Some(observation) = check_observation(summary, arguments, now_unix_ms) {
                    checks.push(observation);
                    if checks.len() > MAX_CHECK_FACTS {
                        checks.remove(0);
                    }
                }
            }
            AgentEvent::ToolDenied { name, .. } if name == "run_command" => {
                pending(&mut requested);
            }
            _ => {}
        }
    }
}

fn check_observation(
    summary: &str,
    arguments: Option<&serde_json::Value>,
    now_unix_ms: i64,
) -> Option<CheckObservation> {
    let (program, code) = parsed_exit(summary)?;
    let raw = arguments
        .map(render_argv)
        .filter(|label| !label.trim().is_empty())
        .unwrap_or_else(|| program.to_owned());
    let label = truncate_chars(&redact(&one_line(&raw)), MAX_CHECK_LABEL_CHARS);
    if label.trim().is_empty() {
        return None;
    }
    Some(CheckObservation {
        label,
        outcome: if code == 0 {
            CheckOutcome::Passed
        } else {
            CheckOutcome::Failed
        },
        tool: "run_command".to_owned(),
        observed_unix_ms: now_unix_ms,
    })
}

/// Consumes the oldest pending request, if any. The request arguments live
/// in the output's events, so the reference outlives the queue's borrow.
fn pending<'a>(requested: &mut Vec<&'a serde_json::Value>) -> Option<&'a serde_json::Value> {
    if requested.is_empty() {
        None
    } else {
        Some(requested.remove(0))
    }
}

/// `{program} exited {code}` — the same summary shape the rendered tool
/// group parses; no other summary records a check.
fn parsed_exit(summary: &str) -> Option<(&str, i64)> {
    let (program, code) = summary.rsplit_once(" exited ")?;
    Some((program, code.parse().ok()?))
}

/// The program and its argv from the request arguments, rendered as one line.
fn render_argv(arguments: &serde_json::Value) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if let Some(program) = arguments.get("program").and_then(serde_json::Value::as_str) {
        parts.push(program);
    }
    if let Some(args) = arguments.get("args").and_then(serde_json::Value::as_array) {
        parts.extend(args.iter().filter_map(serde_json::Value::as_str));
    }
    parts.join(" ")
}

/// Control characters never survive into a label: one line means one line.
fn one_line(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

/// Cut at the bound in characters — never mid-character.
fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}
