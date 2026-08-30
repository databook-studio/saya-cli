//! The mascot drawn on the empty-state splash.

use super::theme::{accent, secondary};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

/// The splash mascot. Every row is padded to the same width so
/// `Alignment::Center` shifts them all by the same amount — a ragged row would
/// centre on its own width and skew the art. The `▌` is the cursor mouth and is
/// styled separately, so it reads as a cursor rather than as more of the body.
const SPLASH_ART: [&str; 8] = [
    "      \u{2588}      ",
    "    \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}    ",
    "  \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}  ",
    "\u{2588}\u{2588}\u{2588}  \u{2588}\u{2588}\u{2588}  \u{2588}\u{2588}\u{2588}",
    "  \u{2588}\u{2588}\u{2588} \u{258c} \u{2588}\u{2588}\u{2588}  ",
    "    \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}    ",
    "      \u{2588}      ",
    "      \u{2591}\u{2591}\u{2591}\u{2591}\u{2591}\u{2591} ",
];

/// The compact mascot, used when the full one would push the splash off-screen.
const SPLASH_ART_COMPACT: [&str; 5] = [
    "    \u{2588}    ",
    "  \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}  ",
    "\u{2588}\u{2588}  \u{2588}  \u{2588}\u{2588}",
    "  \u{2588} \u{258c} \u{2588}  ",
    "    \u{2588}    ",
];

/// The largest mascot that still leaves `content_len` lines of splash text on
/// screen, or `None` when neither fits. A short terminal drops the art rather
/// than pushing the tagline and hints off the top — the mascot is decoration,
/// the text is the thing that has to be read.
pub(super) fn splash_art(
    available_height: usize,
    content_len: usize,
) -> Option<Vec<Line<'static>>> {
    if available_height > content_len + SPLASH_ART.len() {
        Some(art_lines(&SPLASH_ART))
    } else if available_height > content_len + SPLASH_ART_COMPACT.len() {
        Some(art_lines(&SPLASH_ART_COMPACT))
    } else {
        None
    }
}

/// Builds the mascot rows: the body carries the accent, the cast-shadow row
/// recedes into secondary, and the cursor mouth takes the foreground so it
/// reads as a cursor.
fn art_lines(rows: &[&'static str]) -> Vec<Line<'static>> {
    rows.iter()
        .map(|row| {
            // The cast-shadow row is the only one built from the shade glyph.
            if row.contains('\u{2591}') {
                return Line::from(Span::styled(*row, Style::default().fg(secondary())));
            }
            let Some(mouth) = row.find('\u{258c}') else {
                return Line::from(Span::styled(*row, Style::default().fg(accent())));
            };
            let (head, rest) = row.split_at(mouth);
            let (cursor, tail) = rest.split_at('\u{258c}'.len_utf8());
            Line::from(vec![
                Span::styled(head, Style::default().fg(accent())),
                Span::styled(cursor, Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(tail, Style::default().fg(accent())),
            ])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every row must be the same display width, or centring skews the figure.
    #[test]
    fn splash_rows_are_padded_to_one_width() {
        for art in [&SPLASH_ART[..], &SPLASH_ART_COMPACT[..]] {
            let width = art[0].chars().count();
            for row in art {
                assert_eq!(row.chars().count(), width, "ragged row: {row:?}");
            }
        }
    }

    /// The art shrinks, then disappears, rather than pushing the splash text
    /// off the top.
    #[test]
    fn splash_art_yields_to_the_text_when_the_terminal_is_short() {
        // Room to spare: the full mascot.
        assert_eq!(splash_art(40, 11).map(|a| a.len()), Some(SPLASH_ART.len()));
        // One line short of the full mascot: fall back to the compact one.
        assert_eq!(
            splash_art(11 + SPLASH_ART.len(), 11).map(|a| a.len()),
            Some(SPLASH_ART_COMPACT.len())
        );
        // One line short of the compact mascot: the text draws alone.
        assert!(splash_art(11 + SPLASH_ART_COMPACT.len(), 11).is_none());
        assert!(splash_art(0, 11).is_none());
    }

    /// The cursor mouth is its own span so it can be styled apart from the body.
    #[test]
    fn the_cursor_mouth_is_styled_separately() {
        let lines = art_lines(&SPLASH_ART);
        let mouth_row = lines
            .iter()
            .find(|line| line.spans.len() == 3)
            .expect("the mouth row splits into head, cursor, tail");
        assert_eq!(mouth_row.spans[1].content, "\u{258c}");
        assert!(
            mouth_row.spans[1]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }
}
