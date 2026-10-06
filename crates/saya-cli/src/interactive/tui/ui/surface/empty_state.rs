//! The splash / empty state shown before the first conversation turn.

mod state;

use std::borrow::Cow;

use super::super::theme::{accent, secondary, warning};
use super::splash::{NO_WORKSPACE_LINES, splash_art};
use crate::interactive::tui::types::App;
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Paragraph, Wrap},
};

/// The one-line introduction to what this screen leads into: every request
/// becomes a chapter, and a chapter keeps its activity, answer, and results
/// together. Deliberately one line — no tour, no tutorial, no dismissible
/// tip; the first screen is where clutter costs most, and the guidance the
/// splash already carries stays as it is.
const CHAPTER_CONCEPT: &str = "Each request becomes a chapter — activity, answer, and results.";

/// The keyboard hint, the one line that says how to do anything at all.
const KEYBOARD_HINT: &str = "/ commands     @ tables     ? help     Ctrl+C quit";

/// How reluctantly a splash section yields to a short pane, least-kept
/// last. At small sizes decoration goes before guidance, and the stated
/// state of the session outlives the introduction: the keyboard hint never
/// drops, the three readiness lines and the entry choices outlive the
/// concept line, the examples outlive only the splash identity, which goes
/// with the art and the blank spacers.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Tier {
    /// The keyboard hint: how to do anything at all.
    Always,
    /// The three readiness lines, the stated workspace absence, and the
    /// entry choices — state and stated absences, not chrome. Painted
    /// state-first, so the choices shed before the lines on a squeeze.
    State,
    /// The one-line chapter concept.
    Concept,
    /// The `try asking` prompts.
    Examples,
    /// The splash title and tagline: decoration, first to go with the art.
    Decoration,
}

/// One splash block in paint order, tagged with how reluctantly it yields.
/// A section carries the blank row that separates it from the one above, so
/// dropping a section drops its spacer with it.
type Section<'a> = (Tier, Vec<Line<'a>>);

fn line<'a>(text: impl Into<Cow<'a, str>>, style: Style) -> Line<'a> {
    Line::from(Span::styled(text, style))
}

/// Quiet explanatory copy: the tagline, steps, examples, hints, concept.
fn plain<'a>(text: impl Into<Cow<'a, str>>) -> Line<'a> {
    line(text, Style::default().fg(secondary()))
}

/// Stated absences read as a caveat: warning colour, bold.
fn missing<'a>(text: impl Into<Cow<'a, str>>) -> Line<'a> {
    line(
        text,
        Style::default().fg(warning()).add_modifier(Modifier::BOLD),
    )
}

/// The splash title and tagline — decoration, [`Tier::Decoration`].
fn identity() -> Section<'static> {
    (
        Tier::Decoration,
        vec![
            line(
                "◆ saya",
                Style::default().fg(accent()).add_modifier(Modifier::BOLD),
            ),
            plain("Ask your databases in plain language."),
        ],
    )
}

/// The unbound-workspace paragraph, beside — never instead of — the state
/// section above: the absences are orthogonal and both must read.
/// `Some(true)` (a root bound) draws nothing, keeping a bound session's
/// splash free of it.
fn workspace(workspace_bound: Option<bool>) -> Section<'static> {
    if workspace_bound != Some(false) {
        return (Tier::State, Vec::new());
    }
    let mut lines = vec![Line::from("")];
    for line_text in NO_WORKSPACE_LINES {
        lines.push(missing(line_text));
    }
    (Tier::State, lines)
}

/// The one-line naming of the chapter concept, below the state paragraph.
fn concept() -> Section<'static> {
    (Tier::Concept, vec![Line::from(""), plain(CHAPTER_CONCEPT)])
}

/// The `try asking` prompts: what a first request might look like. When the
/// session holds starter questions derived from the active profile's cached
/// schema — computed where the schema tree was already in hand, never here —
/// they replace the static prompts: the user's own tables beat generic copy.
fn examples(app: &App) -> Section<'static> {
    let prompt = |text: String| -> Line<'static> {
        line(
            text,
            Style::default()
                .fg(secondary())
                .add_modifier(Modifier::ITALIC),
        )
    };
    let mut lines: Vec<Line<'static>> = vec![Line::from(""), plain("try asking")];
    if app.starter_questions.is_empty() {
        for text in [
            "  which tables track billing?",
            "  top 5 customers by revenue",
            "  compare row counts across the connected databases",
        ] {
            lines.push(prompt(text.to_string()));
        }
    } else {
        for question in &app.starter_questions {
            lines.push(prompt(format!("  {question}")));
        }
    }
    (Tier::Examples, lines)
}

/// The keyboard hint — [`Tier::Always`], the last thing to go.
fn hint() -> Section<'static> {
    (Tier::Always, vec![Line::from(""), plain(KEYBOARD_HINT)])
}

/// The splash lines that fit `height`: whole sections drop — last-painted
/// first within the least-kept tier — until the rest fits, the keyboard
/// hint always staying. This is `splash_art`'s yield-to-height extended to
/// the text: an explicit drop order instead of wherever the clip falls.
fn visible<'a>(mut kept: Vec<Section<'a>>, height: usize) -> Vec<Line<'a>> {
    kept.retain(|(_, lines)| !lines.is_empty());
    while kept.iter().map(|(_, lines)| lines.len()).sum::<usize>() > height {
        let Some(least_kept) = kept.iter().map(|(tier, _)| *tier).max() else {
            break;
        };
        if least_kept == Tier::Always {
            break; // only the hint is left; the clip has nothing left to take
        }
        let drop_at = kept
            .iter()
            .rposition(|(tier, _)| *tier == least_kept)
            .expect("the tier is present");
        kept.remove(drop_at);
    }
    kept.into_iter().flat_map(|(_, lines)| lines).collect()
}

/// Draws a centered splash/empty state shown before the first conversation turn.
///
/// The state section (in `state`) paints the three readiness lines — AI,
/// project, database — and the entry-choices block; `workspace_bound`
/// decides the project line's presence and the unbound-workspace
/// paragraph, drawn beside — never instead of — the state section, so the
/// orthogonal absences both read.
pub(in crate::interactive::tui) fn draw_empty_state(
    frame: &mut Frame<'_>,
    app: &App,
    area: Rect,
    workspace_bound: Option<bool>,
) {
    let height = area.height as usize;
    let sections = vec![
        identity(),
        (Tier::State, state::state_lines(app, workspace_bound)),
        workspace(workspace_bound),
        (Tier::State, state::choice_lines(workspace_bound)),
        concept(),
        examples(app),
        hint(),
    ];
    // The art yields first: it is admitted only when the *full* splash fits
    // beside it, so it is gone before any text drops. `splash_art` keeps
    // that decision; [`visible`] extends the same yield-to-height to text.
    let full_len = sections.iter().map(|(_, lines)| lines.len()).sum();
    let mut content: Vec<Line<'_>> = visible(sections, height);
    if let Some(with_art) = splash_art(height, full_len) {
        let mut art_and_content: Vec<Line<'_>> = with_art;
        art_and_content.push(Line::from(""));
        art_and_content.extend(content);
        content = art_and_content;
    }

    let content_len = content.len();
    let pad = height.saturating_sub(content_len) / 2;

    let mut lines = Vec::with_capacity(pad + content_len);
    for _ in 0..pad {
        lines.push(Line::from(""));
    }
    lines.extend(content);

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .alignment(Alignment::Center)
            // The state lines are the one place a doctor detail can ride
            // in — an unready provider's line runs past a narrow pane —
            // so over-width lines wrap instead of clipping the remedy
            // away. Trim stays off: the wrapped continuation must keep
            // what the wrap point split, and the label rows never reach
            // the width that would wrap them.
            .wrap(Wrap { trim: false }),
        area,
    );
}

#[cfg(test)]
#[path = "empty_state_tests.rs"]
mod tests;
