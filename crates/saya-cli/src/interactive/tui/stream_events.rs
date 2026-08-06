//! Maps streamed agent events onto transcript blocks.

use super::table;
use super::transcript::{BlockKind, Transcript};
use saya_agent::AgentEvent;

/// Applies one streamed agent event to the transcript.
pub(crate) fn apply_event(transcript: &mut Transcript, event: AgentEvent) {
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
        AgentEvent::ToolRequested { name, arguments } => {
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
        AgentEvent::Complete => {
            transcript.reformat_last(BlockKind::Assistant, table::format_markdown_tables);
        }
    }
}
