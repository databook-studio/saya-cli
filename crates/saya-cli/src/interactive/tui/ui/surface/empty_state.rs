//! The splash / empty state shown before the first conversation turn.

use std::borrow::Cow;

use super::super::theme::{accent, secondary, warning};
use super::splash::{
    NO_DATABASE_FOOTER, NO_DATABASE_HEADLINE, NO_DATABASE_STEPS, NO_WORKSPACE_LINES, splash_art,
};
use crate::interactive::tui::types::App;
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::Paragraph,
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
/// drops, the state paragraph and the stated absences outlive the concept
/// line, the examples outlive only the splash identity, which goes with the
/// art and the blank spacers.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Tier {
    /// The keyboard hint: how to do anything at all.
    Always,
    /// The databases line, or the no-database / no-workspace guidance —
    /// state and stated absences, not chrome.
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

/// The state paragraph: the databases when configured, the no-database
/// guidance when not — a stated absence, not chrome. The copy lives in
/// `splash` so it can be asserted on; see the tests there for what it must
/// hold. The text here once sent new users to `/connect`, which can only
/// select already-configured profiles — a dead end.
fn state<'a>(app: &'a App) -> Section<'a> {
    let mut lines = vec![Line::from("")];
    if app.profiles.is_empty() {
        lines.push(missing(NO_DATABASE_HEADLINE));
        for step in NO_DATABASE_STEPS {
            lines.push(plain(step));
        }
        lines.push(Line::from(""));
        lines.push(plain(NO_DATABASE_FOOTER));
    } else {
        let mut spans = vec![Span::styled(
            "databases  ",
            Style::default().fg(secondary()),
        )];
        for (i, profile) in app.profiles.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled("  ·  ", Style::default().fg(secondary())));
            }
            spans.push(Span::styled(
                profile.as_str(),
                Style::default().fg(accent()),
            ));
        }
        lines.push(Line::from(spans));
    }
    (Tier::State, lines)
}

/// The unbound-workspace paragraph, beside — never instead of — the
/// no-database guidance above: the two absences are orthogonal and both
/// must read. `Some(true)` (a root bound) draws nothing, keeping a bound
/// session's splash byte-identical.
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

/// The `try asking` prompts: what a first request might look like.
fn examples() -> Section<'static> {
    let prompt = |text: &'static str| {
        line(
            text,
            Style::default()
                .fg(secondary())
                .add_modifier(Modifier::ITALIC),
        )
    };
    (
        Tier::Examples,
        vec![
            Line::from(""),
            plain("try asking"),
            prompt("  which tables track billing?"),
            prompt("  top 5 customers by revenue"),
            prompt("  compare row counts across the connected databases"),
        ],
    )
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
/// `workspace_bound` decides the workspace paragraph: `None` (no root
/// bound) draws the unbound line beside — never instead of — the
/// no-database guidance, so the two orthogonal absences both read.
pub(in crate::interactive::tui) fn draw_empty_state(
    frame: &mut Frame<'_>,
    app: &App,
    area: Rect,
    workspace_bound: Option<bool>,
) {
    let height = area.height as usize;
    let sections = vec![
        identity(),
        state(app),
        workspace(workspace_bound),
        concept(),
        examples(),
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
        Paragraph::new(Text::from(lines)).alignment(Alignment::Center),
        area,
    );
}

#[cfg(test)]
#[path = "empty_state_tests.rs"]
mod tests;
