//! One flattened transcript row: the unit every scroll, find, and paint
//! metric derives from. A label row (`YOU`, `SAYA`, …) opens a block; body
//! rows carry the wrapped content.

use super::super::BlockKind;

/// Maps a [`BlockKind`] to its headline label, if it has one.
///
/// `Error`, `System`, and `Thinking` deliberately have no label; `None` is
/// the honest answer for all three, not a placeholder.
///
/// This is the **single** source of the map. It lives here, in the data
/// layer, because `lines()` emits the label rows and `transcript` must not
/// depend on `ui`; the paint step imports it in the other direction, which
/// is the way the dependency already runs (`ui` reads `BlockKind` from
/// here). An earlier packet put a second copy in `ui/labels.rs`; that file
/// is gone, because two maps drift.
pub(crate) fn label(kind: BlockKind) -> Option<&'static str> {
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

/// A single flattened row: `kind` for the rail/style decision, `text` for
/// the content (the label word itself on a label row), `is_label` so find
/// can skip label rows — otherwise a search for "you" lands on every user
/// turn.
#[derive(Debug, Clone)]
pub(crate) struct Row {
    pub(crate) kind: BlockKind,
    pub(crate) text: String,
    pub(crate) is_label: bool,
}

impl Row {
    pub(crate) fn body(kind: BlockKind, text: String) -> Self {
        Self {
            kind,
            text,
            is_label: false,
        }
    }

    pub(crate) fn label(kind: BlockKind) -> Option<Self> {
        label(kind).map(|text| Self {
            kind,
            text: text.to_string(),
            is_label: true,
        })
    }
}

/// The flattened transcript: one entry per painted row, label rows included.
pub(crate) type WrappedLines = Vec<Row>;

/// Wraps one logical line to `width` chars, preferring the last space inside
/// the window so words are not split mid-word; over-long single tokens still
/// split (they have nowhere else to go).
pub(crate) fn wrap_word_aware(raw: &str, width: usize, kind: BlockKind, out: &mut WrappedLines) {
    let chars: Vec<char> = raw.chars().collect();
    let mut start = 0;
    while start < chars.len() {
        let remaining = chars.len() - start;
        if remaining <= width {
            out.push(Row::body(kind, chars[start..].iter().collect()));
            break;
        }
        let window = &chars[start..start + width];
        // Last space in the window (never at position 0, or we would loop).
        let space = window
            .iter()
            .rposition(|c: &char| c.is_whitespace())
            .filter(|&index| index > 0);
        let (emit_end, next_start) = match space {
            // Break on the space: it ends this line (trimmed) and is skipped.
            Some(index) => (index, index + 1),
            None => (width, width),
        };
        out.push(Row::body(
            kind,
            window[..emit_end].iter().collect::<String>(),
        ));
        start += next_start;
    }
}

#[cfg(test)]
mod wrap_tests {
    use super::*;

    fn wrapped_lines(input: &str, width: usize) -> Vec<String> {
        let mut out = Vec::new();
        wrap_word_aware(input, width, BlockKind::System, &mut out);
        out.into_iter().map(|row| row.text).collect()
    }

    #[test]
    fn wraps_on_word_boundaries_when_possible() {
        assert_eq!(
            wrapped_lines("alpha beta gamma", 8),
            vec!["alpha", "beta", "gamma"]
        );
    }

    #[test]
    fn splits_unbreakable_tokens_but_keeps_the_rest_whole() {
        let lines = wrapped_lines("abcdefghij klmno", 6);
        assert_eq!(lines, vec!["abcdef", "ghij", "klmno"]);
    }

    #[test]
    fn short_lines_pass_through_and_leading_space_never_loops() {
        assert_eq!(wrapped_lines("short", 80), vec!["short"]);
        assert_eq!(wrapped_lines("aaaaaaa bbb", 4), vec!["aaaa", "aaa", "bbb"]);
    }
}

#[cfg(test)]
mod label_tests {
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
