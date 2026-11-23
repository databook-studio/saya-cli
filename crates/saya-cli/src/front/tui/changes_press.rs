//! The changes pane's keys, carried out (UI5): opening it on the files the last
//! turn that wrote changed, walking it a row or a page at a time, and closing it.
//!
//! One module per pane's presses, the shape [`super::jobs`] and
//! [`super::overlay_press`] keep: `update.rs` decides which pane a press belongs
//! to and this decides what the pane does with it, so the two never grow into one
//! decision tree.

use super::keys::Intent;
use super::model::Model;

/// How many rows one page of the pane walks: its asked rows, less its two
/// borders and its hint row.
pub(crate) const PAGE: isize = 9;

/// Carries out one press the changes pane owns.
pub(crate) fn press(model: &mut Model, intent: Intent) {
    match intent {
        Intent::OpenChanges => model.open_changes(),
        Intent::ChangesUp => model.scroll_changes(-1),
        Intent::ChangesDown => model.scroll_changes(1),
        Intent::ChangesPageUp => model.scroll_changes(-PAGE),
        Intent::ChangesPageDown => model.scroll_changes(PAGE),
        Intent::ChangesClose => model.close_overlay(),
        // The pane answers these six and nothing else; the update routes no
        // other press here.
        _ => {}
    }
}
