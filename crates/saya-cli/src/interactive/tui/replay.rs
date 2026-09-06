//! Replays saved session turns into the transcript on resume.

use super::super::session_state::SessionState;
use super::table;
use super::transcript::BlockKind;

/// Builds the transcript blocks that represent a resumed session's turns: each
/// turn becomes a user block, a compact line per tool it ran, then the assistant
/// block, ending with a divider noting how much history was replayed.
pub(crate) fn history_blocks(state: &SessionState) -> Vec<(BlockKind, String)> {
    let mut blocks = Vec::new();
    for (i, turn) in state.turns.iter().enumerate() {
        if i > 0 {
            blocks.push((BlockKind::System, String::new()));
        }
        blocks.push((BlockKind::User, turn.user.clone()));
        for tool in &turn.tools {
            let glyph = if tool.status == "completed" {
                "✓"
            } else {
                "✗"
            };
            blocks.push((
                BlockKind::Tool,
                format!("{glyph} {} ({})", tool.name, tool.status),
            ));
            // Show what the call ran: the statement (or arguments) and the
            // value-free result shape. A resumed session can now answer "what
            // did it actually do?" without the live event stream. Cell values
            // are never on the persisted record, so they cannot appear here.
            if let Some(line) = statement_line(&tool.arguments) {
                blocks.push((BlockKind::Tool, line));
            }
            if let Some(line) = shape_line(tool.result_shape.as_ref()) {
                blocks.push((BlockKind::Tool, line));
            }
        }
        blocks.push((
            BlockKind::Assistant,
            table::format_markdown_tables(&turn.assistant),
        ));
    }
    blocks.push((
        BlockKind::System,
        format!(
            "— resumed session {} · {} earlier turn(s) —",
            state.id,
            state.turns.len()
        ),
    ));
    blocks
}

/// Renders the persisted tool-call arguments as a single display line: the SQL
/// statement when the arguments carry one (a SQL tool), otherwise the raw
/// arguments. `None` when the arguments are empty (a tool with no recorded
/// request, e.g. an older session file).
fn statement_line(arguments: &str) -> Option<String> {
    if arguments.trim().is_empty() {
        return None;
    }
    let sql = serde_json::from_str::<serde_json::Value>(arguments)
        .ok()
        .and_then(|value| {
            value
                .get("sql")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        });
    Some(format!("  {}", sql.unwrap_or_else(|| arguments.to_owned())))
}

/// Renders the value-free result shape as a single display line: `→ N rows:
/// col1, col2`. `None` when no shape was recorded (a non-query tool, a denied
/// call, or an older session file).
fn shape_line(shape: Option<&saya_store::RedactedToolResultShape>) -> Option<String> {
    let shape = shape?;
    let noun = if shape.row_count == 1 { "row" } else { "rows" };
    let columns = if shape.columns.is_empty() {
        "no columns".to_owned()
    } else {
        shape.columns.join(", ")
    };
    Some(format!("  → {} {}: {}", shape.row_count, noun, columns))
}

/// Formats a millisecond age as a compact relative time (e.g. "3h ago").
pub(crate) fn relative_time(delta_ms: u128) -> String {
    let secs = delta_ms / 1000;
    if secs < 60 {
        "just now".into()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

#[cfg(test)]
mod tests {
    use super::super::transcript::BlockKind;
    use super::history_blocks;
    use crate::interactive::session_state::SessionState;
    use saya_agent::{ToolMetadata, ToolResultShape};

    #[test]
    fn history_blocks_replays_turns_in_order_with_a_divider() {
        let mut state = SessionState::new("sess-1", None, "m");
        state.record_turn("hello", "hi there", false, vec![]);
        state.record_turn(
            "count users",
            "42 users",
            true,
            vec![ToolMetadata {
                name: "bounded_sql_query".into(),
                status: "completed".into(),
                arguments: r#"{"sql":"SELECT customer, age FROM users"}"#.into(),
                result_shape: Some(ToolResultShape {
                    row_count: 1,
                    columns: vec!["customer".into(), "age".into()],
                }),
            }],
        );

        let blocks = history_blocks(&state);
        // 2 turns → (user, assistant) + spacer + (user, tool, statement, shape,
        // assistant) + a trailing divider.
        assert_eq!(blocks.len(), 9);
        assert_eq!(blocks[0], (BlockKind::User, "hello".to_string()));
        assert_eq!(blocks[1], (BlockKind::Assistant, "hi there".to_string()));
        assert_eq!(blocks[2], (BlockKind::System, String::new()));
        assert_eq!(blocks[3], (BlockKind::User, "count users".to_string()));
        assert_eq!(blocks[4].0, BlockKind::Tool);
        assert!(
            blocks[4].1.contains("bounded_sql_query") && blocks[4].1.contains("completed"),
            "tool line should name the tool and its status: {}",
            blocks[4].1
        );
        // The statement the agent ran is shown on resume.
        assert_eq!(blocks[5].0, BlockKind::Tool);
        assert!(
            blocks[5].1.contains("SELECT customer, age FROM users"),
            "resume should show the statement: {}",
            blocks[5].1
        );
        // The value-free result shape is shown: row count and column names.
        assert_eq!(blocks[6].0, BlockKind::Tool);
        assert!(
            blocks[6].1.contains("1 row") && blocks[6].1.contains("customer, age"),
            "resume should show the result shape: {}",
            blocks[6].1
        );
        assert_eq!(blocks[7], (BlockKind::Assistant, "42 users".to_string()));
        assert_eq!(blocks[8].0, BlockKind::System);
        assert!(
            blocks[8].1.contains("sess-1") && blocks[8].1.contains("2 earlier turn"),
            "divider should name the session and turn count: {}",
            blocks[8].1
        );
    }
}
