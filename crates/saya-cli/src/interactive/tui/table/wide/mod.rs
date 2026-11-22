//! Horizontal clipping for wide result tables. The transcript stores a table
//! block as its full, box-drawn text (what copy and persistence read); this
//! module paints a horizontally-scrolled, optionally column-filtered window of
//! it to the viewport width. The view owns the offset and column state — this
//! code only reads it.

mod clip;
mod columns;
mod geometry;
mod render;

pub(crate) use clip::clip_table_block;

/// Presentation state for wide result tables. This is view state: the block
/// text stays the full, untruncated table (what copy and persistence see), and
/// these fields only change how a table is painted. `h_offset` is the first
/// column index shown in the scroll region; `pin_first` holds column 0 in
/// place while the rest scroll; `columns` restricts the view to named columns
/// (`None` shows all).
///
/// It lived on the old TUI's `App` until X1 deleted that app; the new
/// surface's result pane is this scroller's only reader, so the view state
/// lives in the scroller's module.
#[derive(Debug, Clone, Default)]
pub(crate) struct WideTableView {
    pub(crate) h_offset: usize,
    pub(crate) pin_first: bool,
    pub(crate) columns: Option<Vec<String>>,
}

#[cfg(test)]
#[path = "wide_tests.rs"]
mod tests;
