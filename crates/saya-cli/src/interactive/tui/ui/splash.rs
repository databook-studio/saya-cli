//! What the empty-state splash shows before the first turn: the mascot, and
//! the first-run guidance for someone with no database configured yet.

use super::theme::{accent, secondary};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

/// The headline shown when no connection profile is configured.
pub(super) const NO_DATABASE_HEADLINE: &str = "No database is configured yet.";

/// The steps under that headline.
///
/// Deliberately names no config path. `config init` writes the user config
/// directory by default, `--project` writes `.saya/`, and init prints whichever
/// it chose — so the one place that knows the answer is the command itself.
/// The previous copy hardcoded `.saya/connections.toml` and kept saying it
/// after the default moved, sending first-run users to a file that is not
/// created any more.
pub(super) const NO_DATABASE_STEPS: [&str; 3] = [
    "Run `saya config init` — it prints where it wrote the files.",
    "Add your database there, then `saya connection test <name>`.",
    "Restart saya to pick it up.",
];

/// The closing line, after a blank row.
pub(super) const NO_DATABASE_FOOTER: &str = "`saya config doctor` explains anything still missing.";

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

    /// The guidance must not name a config path. Init's default moved from the
    /// project layer to the user one and this copy kept naming `.saya/`, which
    /// nothing asserted — so it stayed wrong through a release.
    #[test]
    fn first_run_guidance_names_no_config_path() {
        for line in NO_DATABASE_STEPS
            .iter()
            .chain([&NO_DATABASE_HEADLINE, &NO_DATABASE_FOOTER])
        {
            assert!(
                !line.contains(".saya"),
                "guidance must let `config init` report the path: {line}"
            );
        }
    }

    /// Every command the guidance names has to exist, or it is a dead end in the
    /// one place a new user has nothing else to go on.
    #[test]
    fn first_run_guidance_names_real_commands() {
        let all = NO_DATABASE_STEPS.join(" ") + NO_DATABASE_FOOTER;
        for command in [
            "saya config init",
            "saya connection test",
            "saya config doctor",
        ] {
            assert!(all.contains(command), "guidance should offer `{command}`");
        }
    }

    /// The splash is centred, so a line wider than a narrow terminal wraps and
    /// breaks the centring for every line under it.
    #[test]
    fn first_run_guidance_fits_a_narrow_terminal() {
        for line in NO_DATABASE_STEPS
            .iter()
            .chain([&NO_DATABASE_HEADLINE, &NO_DATABASE_FOOTER])
        {
            assert!(line.chars().count() <= 64, "too wide to centre: {line}");
        }
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
