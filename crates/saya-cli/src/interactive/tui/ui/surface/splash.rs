//! What the empty-state splash shows before the first turn: the mascot,
//! and the readiness copy the state section paints — AI, project, and
//! database, each independent — plus the entry choices the first screen
//! offers. The copy lives here so tests can assert on it.

use super::super::theme::{accent, secondary};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

/// The state section's three labels, padded to one width so the three
/// values align down the screen.
pub(in crate::interactive::tui) const AI_LABEL: &str = "AI        ";
pub(in crate::interactive::tui) const PROJECT_LABEL: &str = "Project   ";
pub(in crate::interactive::tui) const DATABASE_LABEL: &str = "Database  ";

/// The AI verdict when the provider is configured; the doctor's provider
/// detail ("ollama, model m") follows it.
pub(in crate::interactive::tui) const AI_READY: &str = "ready — ";

/// The AI verdict when the provider is not configured. The first failing
/// doctor check's detail follows, then the remedy: the AI line carries
/// the instruction, never the database line.
pub(in crate::interactive::tui) const AI_NOT_SET_UP: &str = "not set up — ";
pub(in crate::interactive::tui) const AI_REMEDY: &str = "; try /provider, /model or `saya setup`";

/// The bound-project verdict: with a root bound, coding needs nothing
/// else — the file tools are the difference.
pub(in crate::interactive::tui) const PROJECT_READY: &str = "ready — file tools are available";

/// The no-database verdict: coding does not wait on a database — the
/// line reads as optional and offers the two one-command ways to add one.
pub(in crate::interactive::tui) const DATABASE_NONE: &str =
    "none yet — optional; `saya demo` or `saya setup` adds one";

/// The entry-choices header: the four ways to start, one block.
pub(in crate::interactive::tui) const START_HEADER: &str = "Start with:";

/// The coding choice when a workspace is bound: just ask.
pub(in crate::interactive::tui) const START_CODE_BOUND: &str =
    "Code in this project — just ask, e.g. \"add a test for parse_args\"";

/// The coding choice when no workspace is bound: how to bind one.
pub(in crate::interactive::tui) const START_CODE_UNBOUND: &str =
    "Code in a project — launch inside a git worktree or pass --workspace <dir>";

/// The database choice: guided setup or the read-only demo database.
pub(in crate::interactive::tui) const START_QUERY: &str =
    "Query a database — `saya setup` (guided) or `saya demo` (sample data)";

/// The data-file choice.
pub(in crate::interactive::tui) const START_OPEN: &str =
    "Open a data file — `saya open <file.csv>`";

/// The AI choice: the two commands that show what is available.
pub(in crate::interactive::tui) const START_AI: &str =
    "Change AI — /provider and /model show what is available";

/// The headline shown when no workspace root is bound, beside the
/// state section's own project line when the root is absent: the
/// session names the unbound shape rather than staying silent about
/// it. Two wrapped rows: the centred renderer wraps, it does not clip,
/// so the copy is one logical line rendered as its own paragraph below
/// the state section.
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
