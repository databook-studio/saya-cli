//! The splash's state section: three independent readiness lines — AI,
//! project, database — plus the entry-choices block. Each line states its
//! own fact and carries its own remedy; an absent database reads as
//! optional, never as what stands between the user and their work, and
//! when the AI is not ready its line carries the instruction instead.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::super::super::theme::{accent, secondary, warning};
use super::super::splash::{
    AI_LABEL, AI_NOT_SET_UP, AI_READY, AI_REMEDY, DATABASE_LABEL, DATABASE_NONE, PROJECT_LABEL,
    PROJECT_READY, START_AI, START_CODE_BOUND, START_CODE_UNBOUND, START_HEADER, START_OPEN,
    START_QUERY,
};
use super::plain;
use crate::interactive::tui::startup_readiness::ReadinessLine;
use crate::interactive::tui::types::App;

/// The `Label  content` prefix shared by the three readiness lines: the
/// label quiet, so the values align down the screen.
fn label_span(label: &'static str) -> Span<'static> {
    Span::styled(label, Style::default().fg(secondary()))
}

/// The absence style: warning colour, bold — a stated caveat, not chrome.
fn absence_style() -> Style {
    Style::default().fg(warning()).add_modifier(Modifier::BOLD)
}

/// The AI line: ready with the doctor's provider detail ("ollama, model
/// m"), or not set up with the first failing check's detail and the
/// commands that fix it.
fn ai_line(ai: &ReadinessLine) -> Line<'static> {
    let (content, style) = if ai.ready {
        (
            format!("{AI_READY}{}", ai.detail),
            Style::default().fg(secondary()),
        )
    } else {
        (
            format!("{AI_NOT_SET_UP}{}{AI_REMEDY}", ai.detail),
            absence_style(),
        )
    };
    Line::from(vec![label_span(AI_LABEL), Span::styled(content, style)])
}

/// The database line: the configured profiles joined as today, or the
/// optional absence with the two one-command remedies.
fn database_line<'a>(profiles: &'a [String]) -> Line<'a> {
    let mut spans = vec![label_span(DATABASE_LABEL)];
    if profiles.is_empty() {
        spans.push(Span::styled(DATABASE_NONE, absence_style()));
        return Line::from(spans);
    }
    for (i, profile) in profiles.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ·  ", Style::default().fg(secondary())));
        }
        spans.push(Span::styled(
            profile.as_str(),
            Style::default().fg(accent()),
        ));
    }
    Line::from(spans)
}

/// The state section's lines: AI always, the project line only when a
/// root is bound (the unbound shape is the workspace paragraph beside —
/// never instead — see [`super::workspace`]), database always.
pub(super) fn state_lines<'a>(app: &'a App, workspace_bound: Option<bool>) -> Vec<Line<'a>> {
    let mut lines = vec![Line::from("")];
    lines.push(ai_line(&app.startup.ai));
    if workspace_bound == Some(true) {
        lines.push(Line::from(vec![
            label_span(PROJECT_LABEL),
            Span::styled(PROJECT_READY, Style::default().fg(secondary())),
        ]));
    }
    lines.push(database_line(&app.profiles));
    lines
}

/// The entry-choices block: the four ways to start, fixed copy. It tiers
/// with the state so a short terminal sheds it before the three readiness
/// lines; the coding line's shape follows the workspace binding.
pub(super) fn choice_lines(workspace_bound: Option<bool>) -> Vec<Line<'static>> {
    let code = if workspace_bound == Some(true) {
        START_CODE_BOUND
    } else {
        START_CODE_UNBOUND
    };
    let mut lines = vec![Line::from(""), plain(START_HEADER)];
    for text in [code, START_QUERY, START_OPEN, START_AI] {
        lines.push(plain(text));
    }
    lines
}
