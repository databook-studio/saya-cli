//! Horizontal clipping for wide result tables. The transcript stores a table
//! block as its full, box-drawn text (what copy and persistence read); this
//! module paints a horizontally-scrolled, optionally column-filtered window of
//! it to the viewport width. The view owns the offset and column state — this
//! code only reads it.

mod clip;
mod columns;
mod geometry;
mod names;
mod render;

pub(crate) use clip::clip_table_block;
pub(crate) use names::column_names;

// Re-exported so the `#[path]` sibling test module (which resolves `super::`
// to this module) keeps seeing the names the inline `tests` module saw.
#[cfg(test)]
pub(crate) use crate::interactive::tui::types::WideTableView;

#[cfg(test)]
#[path = "wide_tests.rs"]
mod tests;
