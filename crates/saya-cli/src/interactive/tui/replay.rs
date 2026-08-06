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
    use crate::interactive::session_state::SessionState;
    use super::super::transcript::BlockKind;
    use super::history_blocks;
    use saya_agent::ToolMetadata;

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
            }],
        );

        let blocks = history_blocks(&state);
        // 2 turns → (user, assistant) + spacer + (user, tool, assistant) + a trailing divider.
        assert_eq!(blocks.len(), 7);
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
        assert_eq!(blocks[5], (BlockKind::Assistant, "42 users".to_string()));
        assert_eq!(blocks[6].0, BlockKind::System);
        assert!(
            blocks[6].1.contains("sess-1") && blocks[6].1.contains("2 earlier turn"),
            "divider should name the session and turn count: {}",
            blocks[6].1
        );
    }
}
