//! The surface's overlays, painted over the pane beneath them.
//!
//! One pane owns the screen while it is open (`02` §8): the picker takes the
//! transcript's rows, and nothing else on the frame moves — the status line
//! still names the mode in force, and the composer still holds the draft the
//! operator typed before the picker opened. What a pane says is the pane's own
//! module's business; this one only says where it sits and turns its rows into
//! lines.

use ratatui::layout::Rect;
use ratatui::text::Line;

use super::model::Model;
use super::model_overlay::Overlay;
use super::run_draw::{Facts, lines};
use super::view::trust;
use super::view::{MODE_PICKER_ROWS, TRUST_ROWS, mode_picker};

/// The overlay the model has open, as the rows it covers and the lines it
/// paints there, or nothing when no pane is open.
pub(crate) fn overlay(
    model: &Model,
    facts: &Facts,
    area: Rect,
) -> Option<(Rect, Vec<Line<'static>>)> {
    let overlay = model.overlay()?;
    let rows_wanted = match overlay {
        Overlay::Mode(_) => MODE_PICKER_ROWS,
        Overlay::Trust => TRUST_ROWS,
    };
    let height = (rows_wanted as u16).min(area.height);
    // A pane sits at the bottom of the region it covers, nearest the composer:
    // it is opened by a key press and read on the way out.
    let at = Rect::new(area.x, area.y + area.height - height, area.width, height);
    let rows = match overlay {
        Overlay::Mode(picker) => {
            mode_picker::rows(picker, area.width as usize, height as usize, facts.charset)
        }
        Overlay::Trust => trust::rows(area.width as usize, height as usize, facts.charset),
    };
    Some((at, lines(rows, facts.scheme)))
}
