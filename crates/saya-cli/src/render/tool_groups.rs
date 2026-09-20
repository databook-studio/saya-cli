//! The shared tool-call grouper and shaper: a sequence of [`AgentEvent`] in,
//! a sequence of groups out, and a shaper turning a group into summary text.
//!
//! Pure and deterministic: no clock, no environment reads, no I/O. Both
//! adapters (the piped text renderer and the TUI transcript) consume this one
//! operation; the per-adapter wiring is a later slice.

use std::collections::HashMap;

use saya_agent::{AgentEvent, ToolEffect, read_only_permits};

/// One request/completion pair inside a group.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ToolCallPair {
    pub(crate) name: String,
    pub(crate) arguments: serde_json::Value,
    pub(crate) effect: Option<ToolEffect>,
    pub(crate) summary: Option<String>,
    pub(crate) failed: bool,
    pub(crate) nonzero_exit: bool,
}

/// A maximal run of tool events between two content boundaries.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ToolGroup {
    pub(crate) calls: Vec<ToolCallPair>,
}

/// Failure is the existing contract: the summary keeps the substring
/// "failed", which is what `tool_metadata.status` derives from. This keys on
/// the same predicate, never a second definition.
pub(crate) fn is_failure_summary(summary: &str) -> bool {
    summary.contains("failed")
}

/// The live per-call completion line both TUI render sites mirror onto the
/// tail: `✓` for a completion that succeeded, `✗` for one that failed.
/// Keys on [`is_failure_summary`], never a second definition, so the live
/// tail and the flushed group detail cannot drift — and so a future change
/// to the failure predicate moves both marks together.
pub(crate) fn live_completion_line(name: &str, summary: &str) -> String {
    let mark = if is_failure_summary(summary) {
        "✗"
    } else {
        "✓"
    };
    format!("{mark} {name}: {summary}")
}

/// A nonzero host exit as reported on the piped surface: an `Ok` outcome
/// whose completion summary reads `<program> exited <nonzero>` — no
/// "failed" substring, so `is_failure_summary` (and `tool_metadata.status`,
/// which derives from the same predicate) still reads it as completed. The
/// group shaper consults this separately so the exit code stays visible on
/// the default surface without changing the "failed" contract.
pub(crate) fn is_nonzero_exit_summary(summary: &str) -> bool {
    run_command_exit_code(summary).is_some_and(|code| code != 0)
}

/// The exit code a `run_command` completion summary carries, when it carries
/// one: `<program> exited <code>`. `None` for every other summary shape —
/// including the failure arm (`failed <program>`), which names the call but
/// never the code.
fn run_command_exit_code(summary: &str) -> Option<i64> {
    let (_, code) = summary.rsplit_once(" exited ")?;
    code.parse().ok()
}

/// Splits the event stream into maximal runs of tool events. Every
/// non-member event is a boundary: it closes the open group and is never a
/// member. No time window, no same-tool requirement — ordering defines the
/// run.
pub(crate) fn group_tool_events(events: &[AgentEvent]) -> Vec<ToolGroup> {
    let mut groups = Vec::new();
    let mut current: Vec<ToolCallPair> = Vec::new();
    let mut pending: Vec<(String, serde_json::Value, Option<ToolEffect>)> = Vec::new();

    for event in events {
        match event {
            AgentEvent::ToolRequested {
                name,
                arguments,
                effect,
            } => {
                pending.push((name.clone(), arguments.clone(), *effect));
            }
            AgentEvent::ToolCompleted { name, summary } => {
                let failed = is_failure_summary(summary);
                let nonzero_exit = !failed && is_nonzero_exit_summary(summary);
                let position = pending
                    .iter()
                    .position(|(pending_name, _, _)| pending_name == name);
                let (arguments, effect) = position
                    .map(|index| {
                        let (_, arguments, effect) = pending.remove(index);
                        (arguments, effect)
                    })
                    .unwrap_or_else(|| (serde_json::Value::Null, None));
                current.push(ToolCallPair {
                    name: name.clone(),
                    arguments,
                    effect,
                    summary: Some(summary.clone()),
                    failed,
                    nonzero_exit,
                });
            }
            _ => {
                pending.clear();
                if !current.is_empty() {
                    groups.push(ToolGroup {
                        calls: std::mem::take(&mut current),
                    });
                }
            }
        }
    }
    if !current.is_empty() {
        groups.push(ToolGroup { calls: current });
    }
    groups
}

/// Shapes one group into the lines an adapter renders: today's lines verbatim
/// for a single call, one header for an all-ok group, a header plus one
/// verbatim pair per failed call otherwise. A nonzero host exit is not a
/// `failed` summary (the "failed" substring contract is unchanged), but it is
/// still surfaced: its group carries the exit outcome in the header and
/// prints the call's verbatim pair below, so the code is never dropped from
/// the default surface.
pub(crate) fn shape_group(group: &ToolGroup) -> Vec<String> {
    if group.calls.len() <= 1 {
        return group.calls.iter().flat_map(pair_lines).collect();
    }
    let failed: Vec<&ToolCallPair> = group.calls.iter().filter(|call| call.failed).collect();
    if failed.is_empty() {
        let exited: Vec<&ToolCallPair> = group
            .calls
            .iter()
            .filter(|call| call.nonzero_exit)
            .collect();
        if exited.is_empty() {
            return vec![all_ok_header(group)];
        }
        return nonzero_exit_lines(group, &exited);
    }
    let failures = failed
        .iter()
        .map(|call| failure_label(call))
        .collect::<Vec<_>>()
        .join(", ");
    let mut lines = vec![format!(
        "▸ {} tool calls · {} failed ({}) — details below",
        group.calls.len(),
        failed.len(),
        failures
    )];
    lines.extend(failed.iter().flat_map(|call| pair_lines(call)));
    lines
}

fn request_line(call: &ToolCallPair) -> String {
    let head = if call.effect.as_ref().is_some_and(read_only_permits) {
        format!("Using read-only tool: {}\n", call.name)
    } else {
        format!("Using tool: {}\n", call.name)
    };
    match key_label(&call.name, &call.arguments) {
        Some(detail) => format!("{head}  {detail}\n"),
        None => head,
    }
}

fn completion_line(call: &ToolCallPair) -> String {
    format!("{}: {}\n", call.name, call.summary.as_deref().unwrap_or(""))
}

fn pair_lines(call: &ToolCallPair) -> Vec<String> {
    vec![request_line(call), completion_line(call)]
}

/// The per-call key fact: `run_command` names program + argv, the SQL tools
/// reuse the `tool_call_detail` seam, the write-shaped tools name their key
/// argument. Unknown tools fall back to the bare name.
fn key_label(name: &str, arguments: &serde_json::Value) -> Option<String> {
    if name == "run_command" {
        return run_command_label(arguments);
    }
    if let Some(detail) = crate::agent::tools::tool_call_detail(name, arguments) {
        return Some(detail);
    }
    match name {
        "run_program" => string_argument(arguments, "program"),
        "http_fetch" => string_argument(arguments, "url"),
        "http_download" => string_argument(arguments, "destination"),
        "scratch_sql" => arguments
            .get("sql")
            .and_then(serde_json::Value::as_str)
            .map(|sql| sql.chars().take(24).collect::<String>())
            .filter(|sql| !sql.is_empty()),
        _ => None,
    }
}

fn string_argument(arguments: &serde_json::Value, key: &str) -> Option<String> {
    arguments
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn run_command_label(arguments: &serde_json::Value) -> Option<String> {
    let program = string_argument(arguments, "program")?;
    let mut parts = vec![program];
    if let Some(args) = arguments.get("args").and_then(serde_json::Value::as_array) {
        for arg in args.iter().take(2).filter_map(serde_json::Value::as_str) {
            parts.push(arg.to_owned());
        }
    }
    Some(format!("[{}]", parts.join(" ")))
}

fn all_ok_header(group: &ToolGroup) -> String {
    let mut order: Vec<String> = Vec::new();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut keyed: HashMap<&str, Vec<String>> = HashMap::new();
    for call in &group.calls {
        if !counts.contains_key(call.name.as_str()) {
            order.push(call.name.clone());
            counts.insert(call.name.as_str(), 0);
            keyed.insert(call.name.as_str(), Vec::new());
        }
        if let Some(count) = counts.get_mut(call.name.as_str()) {
            *count += 1;
        }
        if let Some(label) = key_label(&call.name, &call.arguments)
            && let Some(entry) = keyed.get_mut(call.name.as_str())
        {
            entry.push(label);
        }
    }
    let segments = order
        .iter()
        .map(|name| ok_segment(name, &counts, &keyed))
        .collect::<Vec<_>>()
        .join(", ");
    format!("▸ {} tool calls · ok — {segments}", group.calls.len())
}

fn ok_segment(
    name: &str,
    counts: &HashMap<&str, usize>,
    keyed: &HashMap<&str, Vec<String>>,
) -> String {
    let empty = Vec::new();
    let entry = keyed.get(name).unwrap_or(&empty);
    if entry.is_empty() {
        if counts.get(name).copied().unwrap_or(1) > 1 {
            return format!("{name} ×{}", counts[name]);
        }
        return name.to_owned();
    }
    let shown: Vec<&str> = entry.iter().take(3).map(String::as_str).collect();
    let mut segment = format!("{name} {}", shown.join(", "));
    if entry.len() > 3 {
        segment.push_str(&format!(" +{} more", entry.len() - 3));
    }
    segment
}

/// The nonzero-exit group shape: a header carrying the outcome (program and
/// exit code per call) plus one verbatim pair per exited call — the same
/// never-collapse posture as the failure shape, without reclassifying the
/// summary as "failed".
fn nonzero_exit_lines(group: &ToolGroup, exited: &[&ToolCallPair]) -> Vec<String> {
    let outcomes = exited
        .iter()
        .map(|call| call.summary.as_deref().unwrap_or_default().to_owned())
        .collect::<Vec<_>>()
        .join(", ");
    let mut lines = vec![format!(
        "▸ {} tool calls · exited nonzero ({}) — details below",
        group.calls.len(),
        outcomes
    )];
    lines.extend(exited.iter().flat_map(|call| pair_lines(call)));
    lines
}

fn failure_label(call: &ToolCallPair) -> String {
    match key_label(&call.name, &call.arguments) {
        Some(label) if call.name == "run_command" => format!("{} {label}", call.name),
        Some(label) => format!("{} [{label}]", call.name),
        None => call.name.clone(),
    }
}

#[path = "tool_groups_tests.rs"]
#[cfg(test)]
mod tests;
