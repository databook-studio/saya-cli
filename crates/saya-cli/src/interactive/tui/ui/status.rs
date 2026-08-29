//! Status bar rendering.

use super::panels::SPINNER;
use super::theme::{accent, danger, secondary, status_bg, success, warning};
use crate::interactive::session_prompt::StatusView;
use crate::interactive::tui::types::App;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

/// Builds the coloured status-bar segments (profile, provider/model, approval, privacy),
/// each on the bar background so they blend into the strip.
fn status_spans(view: &StatusView, bg: Color) -> Vec<Span<'static>> {
    let base = Style::default().bg(bg);
    let approval_color = match view.approval_mode.as_str() {
        "read-only" => success(),
        "ask" => warning(),
        "never" => danger(),
        _ => secondary(),
    };
    let mut label = view.profile.clone();
    for inc in &view.included {
        label.push_str(&format!(" +{inc}"));
    }
    vec![
        Span::styled(
            format!(" [{label}] "),
            base.fg(accent()).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}/{} ", view.provider, view.model),
            base.fg(secondary()),
        ),
        Span::styled(
            format!("approval:{} ", view.approval_mode),
            base.fg(approval_color),
        ),
        Span::styled(
            format!("privacy:{}", if view.privacy_on { "on" } else { "off" }),
            base.fg(if view.privacy_on {
                success()
            } else {
                secondary()
            }),
        ),
    ]
}

/// Renders the status bar as a filled accent-tinted strip, with a spinner and
/// hint while an agent request is streaming.
pub(super) fn draw_status(frame: &mut Frame<'_>, app: &App, status: &StatusView, area: Rect) {
    let bar = Style::default().bg(status_bg()).fg(secondary());
    let bg = status_bg();
    let line = if app.is_busy() {
        let frame_char = SPINNER[app.spinner % SPINNER.len()];
        let elapsed = app
            .request
            .started
            .map(|start| start.elapsed().as_secs())
            .unwrap_or(0);
        let doing = match &app.request.activity {
            Some(tool) => format!("running {tool} "),
            None => "thinking ".to_string(),
        };
        let mut spans = vec![
            Span::styled(
                format!(" {frame_char} {doing}{elapsed}s "),
                Style::default().bg(bg).fg(accent()),
            ),
            Span::styled("· ", bar),
        ];
        spans.extend(status_spans(status, bg));
        spans.push(Span::styled("  (Esc to cancel) ", bar));
        Line::from(spans)
    } else if app.overlays.selection_mode {
        let mut spans = vec![Span::styled(
            " SELECT ",
            Style::default()
                .bg(accent())
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )];
        spans.extend(status_spans(status, bg));
        spans.push(Span::styled(
            "  ·  drag to copy  ·  Ctrl+O to resume scrolling ",
            bar,
        ));
        Line::from(spans)
    } else {
        let mut spans = status_spans(status, bg);
        spans.push(Span::styled("  ·  ? for help ", bar));
        Line::from(spans)
    };
    frame.render_widget(Paragraph::new(line).style(bar), area);
}
