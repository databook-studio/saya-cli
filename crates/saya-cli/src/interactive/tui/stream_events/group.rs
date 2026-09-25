//! Tool group membership, buffering, and verbatim per-call rendering.

use super::{BlockKind, Transcript};
use saya_agent::AgentEvent;

/// Whether the event is a group member: a request or a completion. Members
/// buffer behind the shared grouper instead of rendering directly.
pub(crate) fn is_group_member(event: &AgentEvent) -> bool {
    matches!(
        event,
        AgentEvent::ToolRequested { .. } | AgentEvent::ToolCompleted { .. }
    )
}

/// Buffers one member event and, while the group is still open, mirrors the
/// per-call line onto the tail block so the running stream stays legible.
/// Returns a boundary event to re-dispatch when the stream's legibility and
/// the buffer disagree (never today: the open group always shows calls live).
///
/// Mirrors one member event onto the tail as today's per-call line (so the
/// running stream stays legible) and buffers the call's facts for the grouper.
/// `TurnReset` discards the buffer: it retries the turn, never completes a run.
///
/// Returns a stray completion that arrived with no open request: it renders as
/// the shared completion line (`✓` for success, `✗` for failure) and stays
/// out of the buffer, so it can never join a group.
pub(crate) fn buffer_tool_event(
    transcript: &mut Transcript,
    event: AgentEvent,
) -> Option<AgentEvent> {
    match event {
        AgentEvent::ToolRequested {
            name,
            arguments,
            effect,
        } => {
            transcript.buffer_tool_request(name, arguments, effect);
            None
        }
        AgentEvent::ToolCompleted { name, summary } => {
            if transcript.buffer_tool_completion(&name, &summary) {
                None
            } else {
                // Stray completion: no open request to pair it with. Render
                // the shared line and keep it out of the group.
                transcript.push(
                    BlockKind::Tool,
                    crate::render::tool_groups::live_completion_line(&name, &summary),
                );
                None
            }
        }
        other => Some(other),
    }
}

/// Folds the buffered run into blocks at the boundary: one collapsed block
/// for a multi-call group, today's verbatim lines for a single call or a
/// group with a failure.
pub(crate) fn flush_tool_buffer(transcript: &mut Transcript) {
    transcript.flush_tool_buffer(
        request_lines,
        crate::render::tool_groups::live_completion_line,
    );
}

/// Today's verbatim request rendering, one block per line: the SQL block or
/// the `→` line, exactly as the TUI rendered before this slice.
fn request_lines(name: &str, arguments: &serde_json::Value) -> Vec<String> {
    if let Some(call) = crate::agent::tools::sql_tool_call(name, arguments) {
        let header = match &call.target {
            Some(t) => format!("SQL · {t}"),
            None => "SQL".to_string(),
        };
        let body = call
            .sql
            .lines()
            .map(|l| format!("  {l}"))
            .collect::<Vec<_>>()
            .join("\n");
        return vec![format!("{header}\n{body}")];
    }
    vec![request_line(name, arguments)]
}

/// Today's verbatim single request line: the SQL block or the `→` line,
/// exactly as the TUI rendered before this slice.
pub(crate) fn request_line(name: &str, arguments: &serde_json::Value) -> String {
    if let Some(call) = crate::agent::tools::sql_tool_call(name, arguments) {
        let header = match &call.target {
            Some(t) => format!("SQL · {t}"),
            None => "SQL".to_string(),
        };
        let body = call
            .sql
            .lines()
            .map(|l| format!("  {l}"))
            .collect::<Vec<_>>()
            .join("\n");
        return format!("{header}\n{body}");
    }
    match tool_call_detail(name, arguments) {
        Some(detail) => format!("→ {name}: {detail}"),
        None => format!("→ {name}"),
    }
}

/// The shared `tool_call_detail` fact, re-exported for the status bar: the
/// bar names the running tool's target with the same words the transcript
/// lines render, so the two cannot drift. A re-export rather than a direct
/// call because `agent::tools` is private to the crate root and invisible
/// from `tui::ui`. Production code calls this — it is not a test seam.
pub(crate) fn tool_call_detail(name: &str, arguments: &serde_json::Value) -> Option<String> {
    crate::agent::tools::tool_call_detail(name, arguments)
}

/// Renders one stray request/completion pair directly, exactly as the stream
/// rendered before grouping. Returns true when the event was a tool edge.
pub(crate) fn apply_tool_edge(transcript: &mut Transcript, event: AgentEvent) -> bool {
    match event {
        AgentEvent::ToolRequested {
            name, arguments, ..
        } => {
            transcript.push(BlockKind::Tool, request_line(&name, &arguments));
            true
        }
        AgentEvent::ToolCompleted { name, summary } => {
            transcript.push(
                BlockKind::Tool,
                crate::render::tool_groups::live_completion_line(&name, &summary),
            );
            true
        }
        AgentEvent::ToolDenied { name, reason } => {
            transcript.push(BlockKind::System, format!("✗ {name} denied: {reason}"));
            true
        }
        _ => false,
    }
}
