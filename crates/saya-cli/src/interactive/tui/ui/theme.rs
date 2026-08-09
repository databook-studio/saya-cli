//! Colour palette and shared style helpers for the TUI.

use crate::interactive::tui::transcript::BlockKind;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
};

/// saya's signature accent (iris violet): brand, assistant, focus, borders.
pub(super) const ACCENT: Color = Color::Rgb(157, 139, 245);
/// User turns — a cool secondary so the accent stays saya's.
pub(super) const USER_COLOR: Color = Color::Rgb(106, 155, 204);
/// Secondary/de-emphasised text: tool + system lines, hints, provider/model.
pub(super) const SECONDARY: Color = Color::Rgb(168, 162, 154);
/// Status: success / safe (read-only approval, privacy on, passing checks).
pub(super) const SUCCESS: Color = Color::Rgb(127, 174, 107);
/// Status: caution (ask approval, approval-panel border).
pub(super) const WARNING: Color = Color::Rgb(224, 164, 88);
/// Status: error / danger (failures, never approval).
pub(super) const DANGER: Color = Color::Rgb(229, 105, 95);
/// Status-bar / badge background (faint iris-tinted dark).
pub(super) const STATUS_BG: Color = Color::Rgb(30, 28, 36);
/// Inline `code` in assistant answers.
pub(super) const CODE_COLOR: Color = Color::Rgb(127, 181, 214);

/// Maps a transcript block kind to its display style.
pub(super) fn kind_style(kind: BlockKind) -> Style {
    match kind {
        BlockKind::User => Style::default().fg(USER_COLOR).add_modifier(Modifier::BOLD),
        BlockKind::Assistant => Style::default().fg(Color::White),
        BlockKind::System => Style::default()
            .fg(SECONDARY)
            .add_modifier(Modifier::ITALIC),
        BlockKind::Error => Style::default().fg(DANGER).add_modifier(Modifier::BOLD),
        BlockKind::Tool => Style::default().fg(SECONDARY),
    }
}

/// Returns the rail color style for a given transcript block kind.
pub(super) fn rail_style(kind: BlockKind) -> Style {
    match kind {
        BlockKind::User => Style::default().fg(USER_COLOR).add_modifier(Modifier::BOLD),
        BlockKind::Assistant => Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        BlockKind::Tool | BlockKind::System => Style::default().fg(SECONDARY),
        BlockKind::Error => Style::default().fg(DANGER).add_modifier(Modifier::BOLD),
    }
}

/// Centers a `width`×`height` rect within `screen`.
pub(super) fn centered(screen: Rect, width: u16, height: u16) -> Rect {
    Rect {
        x: screen.x + (screen.width.saturating_sub(width)) / 2,
        y: screen.y + (screen.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}
