//! The surface's overlays, painted over the pane beneath them.
//!
//! One pane owns the screen while it is open (`02` §8): the picker takes the
//! transcript's rows, the jobs drawer takes its right-hand side or its bottom,
//! and nothing else on the frame moves — the status line still names the mode in
//! force, and the composer still holds the draft the operator typed before the
//! pane opened. What a pane says is the pane's own module's business; this one
//! only says where it sits and turns its rows into lines.

use ratatui::layout::Rect;
use ratatui::text::Line;

use super::model::Model;
use super::model_overlay::Overlay;
use super::run_draw::{Facts, lines};
use super::view::jobs_rows::{Placement, SHEET_ROWS, placement, side_width};
use super::view::{
    Context, MODE_PICKER_ROWS, PALETTE_ROWS, TRUST_ROWS, changes_rows, help_scrolled, jobs_rows,
    mode_picker, palette_rows, sessions_rows, trust,
};

/// The overlay the model has open, as the rows it covers and the lines it
/// paints there, or nothing when no pane is open.
///
/// `screen` is the terminal's own width: `02` §8 decides where the jobs drawer
/// sits by the terminal, not by the content column inside its gutters.
pub(crate) fn overlay(
    model: &Model,
    facts: &Facts,
    area: Rect,
    screen: u16,
) -> Option<(Rect, Vec<Line<'static>>)> {
    let overlay = model.overlay()?;
    let width = area.width as usize;
    let (at, rows) = match overlay {
        Overlay::Mode(picker) => {
            let height = (MODE_PICKER_ROWS as u16).min(area.height);
            (
                anchored(area, height),
                mode_picker::rows(picker, width, height as usize, facts.charset),
            )
        }
        Overlay::Trust => {
            let height = (TRUST_ROWS as u16).min(area.height);
            (
                anchored(area, height),
                trust::rows(width, height as usize, facts.charset),
            )
        }
        // `02` §11's two panes sit where the picker does, for the same reason,
        // and both take the glyph set the frame draws with rather than the
        // UTF-8 default each of them carries.
        Overlay::Palette(palette) => {
            let height = (PALETTE_ROWS as u16).min(area.height);
            let mut pane = palette.clone();
            pane.charset = facts.charset;
            (
                anchored(area, height),
                palette_rows(&pane, width, height as usize),
            )
        }
        Overlay::Help => {
            // The sheet takes the transcript's rows, however many there are: a
            // body taller than the screen scrolls rather than hides a group.
            let height = area.height;
            let context = Context {
                charset: facts.charset,
            };
            (
                area,
                help_scrolled(context, width, height as usize, model.help_scroll()),
            )
        }
        // A drawer with nothing in it is one line, not a panel: the surface is
        // calm about having no jobs.
        Overlay::Jobs if model.jobs().rows.is_empty() => {
            let height = 1.min(area.height);
            (
                anchored(area, height),
                vec![jobs_rows::notice(width, facts.charset)],
            )
        }
        // The changes pane (UI5) takes the transcript's rows while it is open,
        // like the key sheet: what it scrolls is the diff, so it wants room.
        Overlay::Changes(changes) => (
            area,
            changes_rows(
                model.turns(),
                changes,
                width,
                area.height as usize,
                facts.charset,
            ),
        ),
        // The drawer takes the transcript's right-hand side from 100 columns up
        // and a sheet across its bottom below that (`02` §8); the transcript
        // keeps every row and column the drawer does not cover.
        Overlay::Jobs => match placement(screen as usize) {
            Placement::Side => {
                let columns = side_width(width) as u16;
                let side = Rect::new(area.x + area.width - columns, area.y, columns, area.height);
                (
                    side,
                    jobs_rows::rows(
                        model.jobs(),
                        columns as usize,
                        area.height as usize,
                        facts.charset,
                    ),
                )
            }
            Placement::Bottom => {
                let height = (SHEET_ROWS as u16).min(area.height);
                (
                    anchored(area, height),
                    jobs_rows::rows(model.jobs(), width, height as usize, facts.charset),
                )
            }
        },
        // The saved-session picker sits where `/mode`'s pane does, for the same
        // reason: a list read on the way out.
        Overlay::Sessions(picker) => {
            let height = (SESSIONS_ROWS as u16).min(area.height);
            let mut pane = picker.clone();
            pane.charset = facts.charset;
            (
                anchored(area, height),
                sessions_rows(&pane, width, height as usize),
            )
        }
    };
    Some((at, lines(rows, facts.scheme)))
}

/// The rows the saved-session picker takes at most: a list of sessions, a filter
/// and its own borders.
pub(crate) const SESSIONS_ROWS: usize = 14;

/// A pane that sits at the bottom of the region it covers, nearest the composer:
/// it is opened by a key press and read on the way out.
fn anchored(area: Rect, height: u16) -> Rect {
    Rect::new(area.x, area.y + area.height - height, area.width, height)
}
