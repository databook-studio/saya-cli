//! What the empty-state splash shows before the first turn: the mascot, and
//! the first-run guidance for someone with no database configured yet.

use super::super::theme::{accent, secondary};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

/// The headline shown when no connection profile is configured.
pub(crate) const NO_DATABASE_HEADLINE: &str = "No database is configured yet.";

/// The steps under that headline.
///
/// Deliberately names no config path. `config init` writes the user config
/// directory by default, `--project` writes `.saya/`, and init prints whichever
/// it chose — so the one place that knows the answer is the command itself.
/// The previous copy hardcoded `.saya/connections.toml` and kept saying it
/// after the default moved, sending first-run users to a file that is not
/// created any more.
pub(in crate::interactive::tui) const NO_DATABASE_STEPS: [&str; 3] = [
    "Run `saya config init` — it prints where it wrote the files.",
    "Add your database there, then `saya connection test <name>`.",
    "Restart saya to pick it up.",
];

/// The closing line, after a blank row.
pub(in crate::interactive::tui) const NO_DATABASE_FOOTER: &str =
    "`saya config doctor` explains anything still missing.";

/// The headline shown when no workspace root is bound, beside the
/// no-database guidance when both absences hold: the session names the
/// unbound shape rather than staying silent about it. Two wrapped rows:
/// the centred renderer wraps, it does not clip, so the copy is one
/// logical line rendered as its own paragraph below the database guidance.
pub(crate) const NO_WORKSPACE_LINES: [&str; 2] = [
    "No workspace is bound: file tools are unavailable;",
    "launch inside a git worktree or pass `--workspace <dir>`.",
];

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
#[path = "splash_tests.rs"]
mod tests;
