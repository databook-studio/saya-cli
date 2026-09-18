//! Status bar rendering.

use super::panels::SPINNER;
use super::theme::{accent, danger, on_accent, secondary, status_bg, success, warning};
use crate::interactive::session_prompt::StatusView;
use crate::interactive::tui::types::App;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

/// The approval segment's colour, one explicit arm per mode the grammar
/// parses — read-only green (auto-approves reads only), ask amber (a
/// question is pending), never red (everything refuses), bypass red ("every
/// call runs without asking" is the danger it is). The catch-all is a named
/// hole, not a licence: a mode added to `FromStr` but not here would render
/// grey with nothing failing, which is exactly what the colour-map test pins.
fn approval_colour(mode: &str) -> Color {
    match mode {
        "read-only" => success(),
        "ask" => warning(),
        "never" => danger(),
        "bypass" => danger(),
        _ => secondary(),
    }
}

/// Builds the coloured status-bar segments (profile, provider/model, approval,
/// mode, workspace, host, sharing), each on the bar background so they blend
/// into the strip. The `mode:` segment mirrors the headless header's, so the
/// two surfaces cannot drift.
fn status_spans(view: &StatusView, bg: Color) -> Vec<Span<'static>> {
    let base = Style::default().bg(bg);
    let approval_color = approval_colour(&view.approval_mode);
    let mut label = view.profile.clone();
    for inc in &view.included {
        label.push_str(&format!(" +{inc}"));
    }
    let mut spans = vec![
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
        Span::styled(format!("mode:{} ", view.agent_mode), base.fg(secondary())),
    ];
    // The workspace segment names the tree the session can touch, so the
    // binding is visible at every moment it matters — including on a resume
    // from a different directory.
    match view.workspace_root.as_deref() {
        Some(root) => spans.push(Span::styled(format!("ws:{root} "), base.fg(secondary()))),
        None => spans.push(Span::styled("ws:unbound ", base.fg(secondary()))),
    }
    // The host segment names the lane: unsandboxed where the host-command
    // lane composed, off where it did not, plus the denied names where the
    // session's deny list is non-empty — the same words the headless status
    // header carries, so the two surfaces cannot drift.
    let mut host = if view.host_composed {
        "host:unsandboxed".to_string()
    } else {
        "host:off".to_string()
    };
    if !view.denied_programs.is_empty() {
        host.push_str(&format!(" deny:{}", view.denied_programs.join(",")));
    }
    spans.push(Span::styled(format!("{host} "), base.fg(secondary())));
    spans.push(Span::styled(
        format!("sharing:{}", if view.sharing_on { "on" } else { "off" }),
        base.fg(if view.sharing_on {
            warning()
        } else {
            success()
        }),
    ));
    spans
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
                .fg(on_accent())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn bypass_view() -> StatusView {
        StatusView {
            profile: "analytics".into(),
            included: Vec::new(),
            provider: "ollama".into(),
            model: "m".into(),
            approval_mode: "bypass".into(),
            agent_mode: "build".into(),
            workspace_root: None,
            sharing_on: false,
            host_composed: false,
            denied_programs: Vec::new(),
        }
    }

    /// The colour map carries an explicit arm for every mode the grammar
    /// parses; the catch-all (`secondary()`) is the quiet-drift hole a fourth
    /// variant would fall into. `bypass` renders `danger()` red: the mode
    /// that claims "everything runs" must read as the danger it is, in the
    /// grammar's own word, on every surface.
    #[test]
    fn the_status_colour_map_has_an_arm_for_every_mode() {
        for (mode, expected) in [
            ("read-only", success()),
            ("ask", warning()),
            ("never", danger()),
            ("bypass", danger()),
        ] {
            assert_eq!(
                approval_colour(mode),
                expected,
                "{mode} must have its own colour arm"
            );
        }
        assert_eq!(
            approval_colour("whatever-a-future-parse-site-forgot"),
            secondary(),
            "the catch-all is named, not removed: unknown words stay grey"
        );
    }

    /// The two status surfaces agree on the bypass mode: the headless
    /// one-line header renders `approval:bypass`, and the TUI bar renders the
    /// same word in the same `danger()` red — the parity the
    /// `status_segments_mirror_status_line_polarity` precedent pins for
    /// sharing, here for the mode the red indicator belongs to.
    #[test]
    fn the_status_surfaces_render_approval_colon_bypass_in_danger_colour() {
        let mut state = crate::interactive::session_state::SessionState::new(
            "s1",
            Some(String::from("analytics")),
            "m",
        );
        state.approval_mode = "bypass".into();
        let headless = crate::interactive::session_prompt::status_line(&state);
        assert!(
            headless.contains("approval:bypass"),
            "the headless status line says approval:bypass: {headless}"
        );

        let spans = status_spans(&bypass_view(), status_bg());
        let approval = spans
            .iter()
            .find(|span| span.content.starts_with("approval:"))
            .expect("the status bar carries an approval segment");
        assert_eq!(
            approval.content.as_ref(),
            "approval:bypass ",
            "the TUI bar says the same words as the headless line"
        );
        assert_eq!(
            approval.style.fg,
            Some(danger()),
            "bypass renders in danger red, never a softening colour"
        );
    }

    /// The TUI bar carries the `mode:` segment beside `approval:`, with the
    /// same words the headless header renders — the anti-drift check for the
    /// posture `/mode` switches.
    #[test]
    fn the_status_bar_carries_the_mode_segment() {
        let spans = status_spans(&bypass_view(), status_bg());
        let mode = spans
            .iter()
            .find(|span| span.content.starts_with("mode:"))
            .expect("the status bar carries a mode segment");
        assert_eq!(
            mode.content.as_ref(),
            "mode:build ",
            "the TUI bar says the same words as the headless line"
        );
    }
}
