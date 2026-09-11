//! The run panel's rendering: the step list with per-step elapsed times, the
//! live status line, and the episode's transcript — one bordered block docked
//! below the session conversation.
//!
//! Every status word is the shared shaper's output (`run_event_text`), so the
//! panel, the headless wire, and `saya run log` cannot drift; this module
//! only lays lines out. The episode's transcript lines render the same way
//! the conversation's blocks do — kind glyph and kind style from the same
//! theme — but from the panel's own transcript, never the session's.

use super::panels::SPINNER;
use super::theme::{accent, danger, kind_style, rail_style, secondary, success, warning};
use crate::interactive::tui::run_panel::{RunPanel, RunStep, RunStepStatus};
use crate::interactive::tui::transcript::BlockKind;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Paragraph, Wrap},
};

/// How many episode-transcript rows the panel shows (a tail view: the newest
/// lines follow automatically).
const EPISODE_ROWS: usize = 5;

/// The panel's height: one row per plan step (or one while the plan is being
/// proposed), the status line, the episode tail, and the border — clamped so
/// the conversation above keeps at least half the screen.
pub(super) fn run_panel_height(panel: &RunPanel, height: u16) -> u16 {
    let rows = panel.steps.len().max(1) as u16 + 1 + EPISODE_ROWS as u16 + 2;
    rows.clamp(8, (height / 2).max(8))
}

/// One step list row: the one-based step, its goal, and where it stands —
/// pending, running with its live elapsed, or done/failed with the elapsed
/// the boundary event froze.
fn step_line(index: usize, step: &RunStep) -> Line<'static> {
    let one_based = index + 1;
    let goal = step.goal.clone();
    let elapsed = step
        .elapsed
        .map(|d| format!(" {}s", d.as_secs()))
        .unwrap_or_default();
    let (mark, style) = match step.status {
        RunStepStatus::Pending => ("·".to_string(), Style::default().fg(secondary())),
        RunStepStatus::Running => ("▶".to_string(), Style::default().fg(accent())),
        RunStepStatus::Completed => ("✓".to_string(), Style::default().fg(success())),
        RunStepStatus::Failed => ("✗".to_string(), Style::default().fg(danger())),
    };
    Line::from(vec![
        Span::styled(format!("{mark} "), style),
        Span::styled(
            format!("{one_based}. {goal}"),
            Style::default().fg(secondary()),
        ),
        Span::styled(elapsed, style.add_modifier(Modifier::BOLD)),
    ])
}

/// Draws the run panel into its docked area: step list, live status, and the
/// episode's transcript tail.
pub(super) fn draw_run_panel(
    frame: &mut Frame<'_>,
    app: &crate::interactive::tui::types::App,
    area: Rect,
) {
    let Some(panel) = app.run_panel.as_ref() else {
        return;
    };
    let inner_width = area.width.saturating_sub(2).max(1) as usize;
    let phase = if panel.cancelling {
        "cancelling…".to_string()
    } else if panel.status.is_empty() && !panel.terminated {
        "starting…".to_string()
    } else {
        panel.status.clone()
    };
    let title_goal: String = panel.goal.chars().take(40).collect();
    let title = if panel.goal.is_empty() {
        format!(" run {} ", panel.run_id)
    } else {
        format!(" run {} · {title_goal} ", panel.run_id)
    };
    let block = Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(accent()))
        .title(Span::styled(
            title,
            Style::default().fg(accent()).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    if panel.steps.is_empty() && !panel.terminated {
        lines.push(Line::from(Span::styled(
            "· proposing a plan…",
            Style::default().fg(secondary()),
        )));
    } else {
        for (index, step) in panel.steps.iter().enumerate() {
            lines.push(step_line(index, step));
        }
    }
    lines.push(Line::from(""));
    lines.push(status_line(panel, phase));
    let episode_rows = inner.height.saturating_sub(lines.len() as u16).max(1) as usize;
    for (kind, text) in panel.episode.view(inner_width, episode_rows) {
        if text.is_empty() {
            lines.push(Line::from(""));
            continue;
        }
        lines.push(Line::from(vec![
            Span::styled(glyph(kind), rail_style(kind)),
            Span::styled(text, kind_style(kind)),
        ]));
    }
    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
        inner,
    );
}

/// The live status line: a spinner and the run's elapsed seconds while it is
/// in flight, the last lifecycle line once something has happened, and the
/// Esc affordance while the run can be cancelled.
fn status_line(panel: &RunPanel, phase: String) -> Line<'static> {
    let mut spans = Vec::new();
    if panel.is_active() {
        let frame_char = SPINNER[panel.spinner % SPINNER.len()];
        let secs = panel.started.map(|s| s.elapsed().as_secs()).unwrap_or(0);
        spans.push(Span::styled(
            format!("{frame_char} "),
            Style::default().fg(accent()),
        ));
        spans.push(Span::styled(
            if phase.is_empty() {
                format!("run · {secs}s")
            } else {
                format!("{phase} · {secs}s")
            },
            style_of(panel),
        ));
        if !panel.cancelling {
            spans.push(Span::styled(
                " · Esc cancels",
                Style::default().fg(secondary()),
            ));
        }
    } else {
        spans.push(Span::styled(
            if phase.is_empty() {
                "run ended".to_string()
            } else {
                phase
            },
            style_of(panel),
        ));
        spans.push(Span::styled(
            " · Esc closes",
            Style::default().fg(secondary()),
        ));
    }
    Line::from(spans)
}

fn style_of(panel: &RunPanel) -> Style {
    if panel.status_is_error {
        Style::default().fg(danger())
    } else if panel.paused {
        // A pause is resumable, never a success or a failure: the colour says
        // "attention", the line says why.
        Style::default().fg(warning())
    } else if panel.terminated {
        Style::default().fg(success())
    } else {
        Style::default().fg(accent())
    }
}

/// The episode block's rail glyph — the same glyphs the conversation's
/// transcript renders, so an episode reads like any other stream.
fn glyph(kind: BlockKind) -> &'static str {
    match kind {
        BlockKind::User => "❯ ",
        BlockKind::Assistant => "◆ ",
        BlockKind::Tool | BlockKind::Table => "▸ ",
        BlockKind::Error => "✗ ",
        BlockKind::System => "· ",
        BlockKind::Thinking => "≈ ",
    }
}
