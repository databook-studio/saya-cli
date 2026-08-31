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

/// The splash mascot: an owl whose pupils are terminal cursors.
///
/// Every row is padded to the same width so `Alignment::Center` shifts them all
/// by the same amount — a ragged row centres on its own width and skews the
/// figure. The pupils converge (`▐` in the left socket, `▌` in the right) so the
/// owl reads as focused rather than vacant; the sockets themselves are gaps, so
/// when a pupil is absent the eye reads as closed rather than as a hole.
const SPLASH_ART: [&str; 8] = [
    "   \u{2584}\u{2584}       \u{2584}\u{2584}   ",
    "   \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}   ",
    " \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588} ",
    " \u{2588}\u{2588}\u{2588} \u{2590} \u{2588}\u{2588}\u{2588} \u{258c} \u{2588}\u{2588}\u{2588} ",
    " \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{25bc}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588} ",
    "  \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}  ",
    "    \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}    ",
    "     \u{2580}\u{2580}   \u{2580}\u{2580}     ",
];

/// The compact mascot, used when the full one would push the splash off-screen.
const SPLASH_ART_COMPACT: [&str; 5] = [
    "  \u{2584}     \u{2584}  ",
    "  \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}  ",
    " \u{2588}\u{2588} \u{2590} \u{258c} \u{2588}\u{2588} ",
    " \u{2588}\u{2588}\u{2588}\u{2588}\u{25bc}\u{2588}\u{2588}\u{2588}\u{2588} ",
    "  \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}  ",
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

/// How one glyph of the art is painted.
///
/// The art only ever draws the idle face, so `█` is unambiguously body: the
/// states that would reuse it as a pupil are not rendered here. Were that to
/// change, the pupils would need their own glyphs rather than a shared one.
fn glyph_style(ch: char) -> Style {
    match ch {
        // The pupils are the cursor — the one part the state system substitutes.
        '\u{258c}' | '\u{2590}' => Style::default().add_modifier(Modifier::BOLD),
        // Beak and talons recede so the eyes stay the focus.
        '\u{25bc}' | '\u{2580}' => Style::default().fg(secondary()),
        _ => Style::default().fg(accent()),
    }
}

/// Builds the mascot rows, coalescing runs of same-styled glyphs so a row is a
/// handful of spans rather than one per cell.
///
/// This walks the row instead of splitting on a single cursor: the owl has two
/// pupils, and the previous single-split version silently dropped everything
/// after the first one.
fn art_lines(rows: &[&'static str]) -> Vec<Line<'static>> {
    rows.iter()
        .map(|row| {
            let mut spans: Vec<Span<'static>> = Vec::new();
            for ch in row.chars() {
                let style = glyph_style(ch);
                match spans.last_mut() {
                    Some(last) if last.style == style => last.content.to_mut().push(ch),
                    _ => spans.push(Span::styled(ch.to_string(), style)),
                }
            }
            Line::from(spans)
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

    /// Both pupils must survive styling. The previous builder split the row on
    /// the first cursor glyph and emitted three spans, which silently dropped
    /// the second eye the moment the mascot grew one.
    #[test]
    fn both_pupils_are_styled_as_cursors() {
        for art in [&SPLASH_ART[..], &SPLASH_ART_COMPACT[..]] {
            let lines = art_lines(art);
            let bold: Vec<String> = lines
                .iter()
                .flat_map(|line| &line.spans)
                .filter(|span| span.style.add_modifier.contains(Modifier::BOLD))
                .map(|span| span.content.to_string())
                .collect();
            assert_eq!(
                bold,
                vec!["\u{2590}", "\u{258c}"],
                "expected exactly two pupils"
            );
        }
    }

    /// A row must round-trip: coalescing runs may change how the text is split
    /// into spans, never which characters reach the screen.
    #[test]
    fn styling_preserves_every_glyph_in_the_row() {
        for art in [&SPLASH_ART[..], &SPLASH_ART_COMPACT[..]] {
            for (row, line) in art.iter().zip(art_lines(art)) {
                let rendered: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                assert_eq!(&rendered, row, "row must survive styling unchanged");
            }
        }
    }

    /// Beak and talons recede; the body carries the accent. Without this a glyph
    /// added to the art silently inherits the body colour.
    #[test]
    fn beak_and_talons_recede_behind_the_body() {
        let secondary_style = Style::default().fg(secondary());
        for ch in ['\u{25bc}', '\u{2580}'] {
            assert_eq!(glyph_style(ch), secondary_style, "{ch} should recede");
        }
        assert_eq!(glyph_style('\u{2588}'), Style::default().fg(accent()));
    }
}
