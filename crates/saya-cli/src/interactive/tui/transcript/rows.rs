//! One flattened transcript row: the unit every scroll, find, and paint
//! metric derives from. A label row (`YOU`, `SAYA`, …) opens a block; body
//! rows carry the wrapped content.

use super::BlockKind;

/// Maps a [`BlockKind`] to its headline label, if it has one.
///
/// `Error`, `System`, and `Thinking` deliberately have no label; `None` is
/// the honest answer for all three, not a placeholder. This duplicates
/// `ui::labels::label` (which that packet's paint step consumes) so the data
/// layer stays paint-free: `transcript` must not depend on `ui`.
fn label(kind: BlockKind) -> Option<&'static str> {
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
