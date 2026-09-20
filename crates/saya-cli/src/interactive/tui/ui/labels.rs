//! Word labels for transcript block kinds (Fieldnotes Phase 2).
//!
//! The glyph rail stays the rendered output; this map is consumed by later
//! packets and changes nothing on its own.

use crate::interactive::tui::transcript::BlockKind;

/// Maps a [`BlockKind`] to its headline label, if it has one.
///
/// `Error`, `System`, and `Thinking` deliberately have no label yet; `None`
/// is the honest answer for all three, not a placeholder.
#[allow(dead_code)]
pub(super) fn label(kind: BlockKind) -> Option<&'static str> {
    match kind {
        BlockKind::User => Some("YOU"),
        BlockKind::Assistant => Some("SAYA"),
        BlockKind::Tool => Some("ACTIVITY"),
        BlockKind::Table => Some("RESULT"),
        BlockKind::Error => None,
        BlockKind::System => None,
        BlockKind::Thinking => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assistant_maps_to_saya_while_your_choice_maps_to_nothing() {
        assert_eq!(label(BlockKind::Assistant), Some("SAYA"));
        for kind in [
            BlockKind::User,
            BlockKind::Assistant,
            BlockKind::Tool,
            BlockKind::Table,
            BlockKind::Error,
            BlockKind::System,
            BlockKind::Thinking,
        ] {
            let text = label(kind).unwrap_or("");
            assert_ne!(text, "YOUR CHOICE", "{kind:?} must not map to YOUR CHOICE");
            assert_ne!(text, "PARTIAL", "{kind:?} must not map to PARTIAL");
            assert_ne!(text, "WORKING", "{kind:?} must not map to WORKING");
        }
    }

    #[test]
    fn every_kind_is_mapped_explicitly() {
        assert_eq!(label(BlockKind::User), Some("YOU"));
        assert_eq!(label(BlockKind::Assistant), Some("SAYA"));
        assert_eq!(label(BlockKind::Tool), Some("ACTIVITY"));
        assert_eq!(label(BlockKind::Table), Some("RESULT"));
        assert_eq!(label(BlockKind::Error), None);
        assert_eq!(label(BlockKind::System), None);
        assert_eq!(label(BlockKind::Thinking), None);
    }

    #[test]
    fn a_finished_tool_call_is_not_labelled_as_running_work() {
        // Tool blocks include completed calls, and the design states that
        // "Working becomes Complete only when the stated work is actually
        // complete" — labelling a finished call WORKING would break that
        // rule, so the trail of operations reads ACTIVITY instead.
        assert_eq!(label(BlockKind::Tool), Some("ACTIVITY"));
        assert_ne!(label(BlockKind::Tool), Some("WORKING"));
    }
}
