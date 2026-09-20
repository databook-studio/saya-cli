//! Tool-group buffering entry: membership test, member buffering, flush,
//! and the boundary events that split groups (moved verbatim from
//! `stream_events.rs`: `is_group_member`, `buffer_tool_event`,
//! `flush_tool_buffer`, `request_lines`, `tool_call_detail`).

use super::super::transcript::{BlockKind, Transcript};
use saya_agent::AgentEvent;

/// Applies one streamed agent event to the transcript.
///
/// `show_thinking` gates whether the model's chain-of-thought reaches the
/// transcript: off by default, so a user who did not ask for it never sees it.
/// When on, reasoning is pushed as a dimmed `Thinking` block — visually
/// subordinate to the answer, never mistakable for it. Either way reasoning is
/// in-memory only and never persisted.
///
/// Tool events buffer behind the shared grouper and flush at the next boundary
/// event: a run of tool calls lands as one collapsed block carrying the
/// Decision-2 summary (the same string the piped surface emits), expandable to
/// today's per-call `→` / `✓` lines. Streaming with the tail followed shows the
/// per-call lines as they arrive (a collapse imposed mid-stream would rewrite
/// history the user just watched); the group collapses when the boundary event
/// that ends it arrives.
pub(crate) fn apply_event(transcript: &mut Transcript, event: AgentEvent, show_thinking: bool) {
    // A caller that ends the stream after a tool run (tests, the panel's
    // final `Complete`) flushes on the boundary below. A caller that stops
    // mid-run with no boundary leaves buffered calls unrendered, so a trailing
    // flush would misattribute the next turn's text as this group's boundary —
    // keep the buffer, don't flush it here.
    if is_group_member(&event) {
        if let Some(other) = buffer_tool_event(transcript, event) {
            apply_boundary_event(transcript, other, show_thinking);
        }
        return;
    }
    // The retry discards the run in flight: the failure it reports belongs to
    // the transport, never to a collapsed summary.
    if matches!(event, AgentEvent::TurnReset) {
        transcript.discard_tool_buffer();
    } else {
        flush_tool_buffer(transcript);
    }
    apply_boundary_event(transcript, event, show_thinking);
}

/// Buffers one member event and, while the group is still open, mirrors the
/// per-call line onto the tail block so the running stream stays legible.
/// Returns a boundary event to re-dispatch when the stream's legibility and
/// the buffer disagree (never today: the open group always shows calls live).
fn is_group_member(event: &AgentEvent) -> bool {
    matches!(
        event,
        AgentEvent::ToolRequested { .. } | AgentEvent::ToolCompleted { .. }
    )
}

/// Mirrors one member event onto the tail as today's per-call line (so the
/// running stream stays legible) and buffers the call's facts for the grouper.
/// `TurnReset` discards the buffer: it retries the turn, never completes a run.
///
/// Returns a stray completion that arrived with no open request: it renders as
/// today's `✓` line and stays out of the buffer, so it can never join a group.
fn buffer_tool_event(transcript: &mut Transcript, event: AgentEvent) -> Option<AgentEvent> {
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
                // today's line and keep it out of the group.
                transcript.push(BlockKind::Tool, format!("✓ {name}: {summary}"));
                None
            }
        }
        other => Some(other),
    }
}

/// Folds the buffered run into blocks at the boundary: one collapsed block
/// for a multi-call group, today's verbatim lines for a single call or a
/// group with a failure.
fn flush_tool_buffer(transcript: &mut Transcript) {
    transcript.flush_tool_buffer(request_lines, |name, summary| {
        format!("✓ {name}: {summary}")
    });
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
    vec![
        match crate::agent::tools::tool_call_detail(name, arguments) {
            Some(detail) => format!("→ {name}: {detail}"),
            None => format!("→ {name}"),
        },
    ]
}

/// The shared `tool_call_detail` fact, re-exported for the status bar: the
/// bar names the running tool's target with the same words the transcript
/// lines render, so the two cannot drift. A re-export rather than a direct
/// call because `agent::tools` is private to the crate root and invisible
/// from `tui::ui`. Production code calls this — it is not a test seam.
pub(crate) fn tool_call_detail(name: &str, arguments: &serde_json::Value) -> Option<String> {
    crate::agent::tools::tool_call_detail(name, arguments)
}

fn apply_boundary_event(transcript: &mut Transcript, event: AgentEvent, show_thinking: bool) {
    match event {
        AgentEvent::AssistantText { text } => {
            if !matches!(
                transcript.blocks().last().map(|b| b.kind),
                Some(BlockKind::Assistant)
            ) {
                // first chunk of the answer: separate it from the tool/SQL lines above
                if transcript
                    .blocks()
                    .last()
                    .is_some_and(|b| !b.text.is_empty())
                {
                    transcript.push(BlockKind::System, String::new());
                }
            }
            transcript.append_delta(BlockKind::Assistant, &text);
        }
        // The provider stream failed mid-answer and the loop is retrying the
        // turn. The text streamed so far is discarded: clear the trailing
        // assistant block (the one the next delta would extend) so the
        // re-streamed answer replaces it instead of appending to it.
        AgentEvent::TurnReset => transcript.reset_delta(BlockKind::Assistant),
        AgentEvent::ToolRequested {
            name, arguments, ..
        } => {
            if let Some(call) = crate::agent::tools::sql_tool_call(&name, &arguments) {
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
                let text = format!("{header}\n{body}");
                transcript.push(BlockKind::Tool, text);
            } else {
                let line = match crate::agent::tools::tool_call_detail(&name, &arguments) {
                    Some(detail) => format!("→ {name}: {detail}"),
                    None => format!("→ {name}"),
                };
                transcript.push(BlockKind::Tool, line);
            }
        }
        AgentEvent::ToolCompleted { name, summary } => {
            transcript.push(BlockKind::Tool, format!("✓ {name}: {summary}"))
        }
        AgentEvent::ToolDenied { name, reason } => {
            transcript.push(BlockKind::System, format!("✗ {name} denied: {reason}"))
        }
        // What memory supplied, shown before the answer streams. The shared
        // shaper centralizes the wording; an empty
        // result (Ran-and-found-nothing) is silence — push nothing.
        AgentEvent::KnowledgeSupplied {
            outcome,
            contracts,
            dropped_by_bounds,
        } => {
            let text =
                crate::render::knowledge_supplied_text(outcome, &contracts, dropped_by_bounds);
            if !text.is_empty() {
                // Strip the trailing newline: the transcript stores line text
                // without a delimiter and re-wraps per line; a trailing '\n' would
                // push a blank line into the block.
                transcript.push(BlockKind::System, text.trim_end_matches('\n'));
            }
        }
        // A confirmed claim the turn's SQL contradicted. Trails the
        // answer — emitted after the loop — so a System block pushed here lands
        // below the assistant text, where a "the SQL contradicted a confirmed
        // claim" notice belongs. The shared shaper centralizes the wording; an
        // empty finding set is silence (the runtime emits nothing, but this
        // guards a directly-constructed event too).
        AgentEvent::KnowledgeOverridden { findings } => {
            let text = crate::render::knowledge_overridden_text(&findings);
            if !text.is_empty() {
                transcript.push(BlockKind::System, text.trim_end_matches('\n'));
            }
        }
        // Extraction timed out or errored after the turn succeeded (spec
        // packet-54). Trails the answer — emitted after the loop — so a System
        // block pushed here lands below the assistant text, where "and I did
        // not learn from this turn" belongs. Shares the shaper with the
        // headless path so the wording lives in one place; the line is never
        // empty for a known reason, so the block always pushes.
        AgentEvent::KnowledgeLearningSkipped { reason } => {
            let text = crate::render::learning_skipped_text(reason);
            if !text.is_empty() {
                transcript.push(BlockKind::System, text.trim_end_matches('\n'));
            }
        }
        // One fact learned this turn. Trails the answer — the runtime emits it
        // after the loop — so it lands below the assistant text, where "and I
        // kept this" belongs. Shares the shaper with the headless path; an
        // undescribable claim is silence, never a raw token.
        AgentEvent::KnowledgeProposed { claim } => {
            let text = crate::render::knowledge_learned_text(&claim);
            if !text.is_empty() {
                transcript.push(BlockKind::System, text.trim_end_matches('\n'));
            }
        }
        // The model's chain-of-thought. Shown only when the user opted in; otherwise
        // accepted and dropped, so the event never reaches the catch-all and never
        // renders as an error. When shown it lands as a dimmed `Thinking` block,
        // separate from the assistant answer and visually subordinate to it. Reasoning
        // is in-memory only: the transcript is never serialized, and the persisted
        // session types carry role + content only, so holding it here cannot reach a
        // session file regardless of the display toggle.
        AgentEvent::ReasoningText { text } => {
            if show_thinking && !text.is_empty() {
                transcript.push(BlockKind::Thinking, text);
            }
        }
        // The token counts one provider call reported. Accepted and dropped:
        // the transcript already shows a per-turn token line from the run
        // output at `Done`, and `/usage` breaks the session down, so a block
        // here would duplicate them. The event exists for the JSON/NDJSON
        // boundary; it must not reach the catch-all and disappear silently
        // into an error.
        AgentEvent::Usage { .. } => {}
        AgentEvent::Complete => {
            transcript.reformat_last(BlockKind::Assistant, table::format_markdown_tables);
        }
        // Silent bookkeeping the grouper must still treat as a boundary.
        // `Usage` arrives after the answer finished streaming and carries no
        // content — but a group cannot span it, exactly as the piped adapter
        // flushes on every non-member event including silent ones.
        AgentEvent::KnowledgeLearningStarted => {}
        _ => {}
    }
}
