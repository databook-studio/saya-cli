//! The look the new surface reuses: the reading surface's renderers, and the
//! theme's hues and palette choice.
//!
//! The frame the old surface drew — the context line, the status bar, the input
//! box, the overlays and the transcript pane — went with it in X1; the new
//! surface paints its own from `front/tui/view` and `front/tui/run_draw`.

pub(crate) mod surface;
pub(crate) mod theme;
