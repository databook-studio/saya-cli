//! The mode picker (`02` §8): the six modes of the one consent axis, the row
//! the operator has chosen, and the pane that paints them.
//!
//! A value and a pure painter. The value is what `/mode` with no argument opens
//! and what the keymap walks ([`super::super::model_overlay::Overlay`]); the
//! painter is one function from the picker, a width and a height to the rows a
//! pane paints, so the whole picker is asserted without a terminal. The box and
//! its cell arithmetic are [`super::palette_frame`]'s, reused rather than
//! copied.
//!
//! Nothing here switches a mode: the row an operator sets becomes a
//! [`Cmd::SetMode`](crate::front::tui::msg::Cmd::SetMode) the runtime performs
//! through the chat layer's one author of a mode switch. The words a mode says
//! live beside this file, in [`super::mode_words`], so the pane, the toast and
//! the transcript read one table.

use saya_agent::PermissionMode;

use super::composer::{Row, Run};
use super::mode_words::MODES;
use super::palette_frame::{MIN_WIDTH, content, frame};
use crate::front::tui::theme::{Charset, Glyph, Token, glyph};
use crate::interactive::session::permissions::mode::name;
use crate::interactive::tui::wrap::cell_width;

/// The pane's words, read through here: the picker draws them and says them.
#[allow(unused_imports)]
pub(crate) use super::mode_words::{TITLE, keys, line, toast, words};

/// The rows the pane is drawn in: its top border, the six modes, its bottom
/// border.
pub(crate) const ROWS: usize = MODES.len() + 2;

/// The cells between a row's name and its words.
const GAP: usize = 2;

/// The mark of the mode in force. `●` is the theme's bullet, which `02` §12
/// pairs with a colour and a word elsewhere; here the glyph is the whole
/// meaning, so the mark is named beside the rows it sits on.
const CURRENT: Glyph = Glyph::Bullet;

/// The picker: the row selected, and the mode in force when it opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModePicker {
    /// The selected row's index into [`MODES`].
    pub(crate) selected: usize,
    /// The mode in force, marked on its row and never moved by the picker.
    pub(crate) current: PermissionMode,
}

impl ModePicker {
    /// The picker over `current`, with the mode in force already selected.
    pub(crate) fn on(current: PermissionMode) -> Self {
        Self {
            selected: row_of(current),
            current,
        }
    }

    /// The mode the selected row names.
    pub(crate) fn chosen(&self) -> PermissionMode {
        MODES[self.selected.min(MODES.len() - 1)].0
    }

    /// Moves the selection one row down, and no further than the last row.
    pub(crate) fn down(&mut self) {
        self.selected = (self.selected + 1).min(MODES.len() - 1);
    }

    /// Moves the selection one row up, and no further than the first row.
    pub(crate) fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }
}

/// The pane at `width` columns and `height` rows, top border first.
///
/// Exactly `height` rows come back: a pane taller than the picker is padded
/// rather than short, and one too short to hold the six rows and its keys is
/// cut at its height instead of spilling.
pub(crate) fn rows(picker: &ModePicker, width: usize, height: usize, charset: Charset) -> Vec<Row> {
    if height == 0 {
        return Vec::new();
    }
    let width = width.max(MIN_WIDTH);
    let mut out = vec![top(width, charset)];
    for (index, (mode, words)) in MODES.iter().enumerate() {
        out.push(content(
            row(picker, index, *mode, words, charset),
            width,
            charset,
        ));
    }
    while out.len() < height - 1 {
        out.push(content(Vec::new(), width, charset));
    }
    out.push(bottom(width, charset));
    out.truncate(height);
    out
}

/// One mode's row: the selection mark, the mark of the mode in force, its name
/// and the one line it means.
fn row(
    picker: &ModePicker,
    index: usize,
    mode: PermissionMode,
    words: &str,
    charset: Charset,
) -> Vec<Run> {
    let selected = index == picker.selected;
    let token = if selected { Token::Focus } else { Token::Fg };
    let marker = if selected {
        glyph(Glyph::Selected, charset)
    } else {
        " "
    };
    let current = if mode == picker.current {
        glyph(CURRENT, charset)
    } else {
        " "
    };
    let name = name(mode);
    let pad = " ".repeat(name_column().saturating_sub(cell_width(name)) + GAP);
    vec![
        Run {
            text: format!("{marker} {current} {name}{pad}"),
            token,
        },
        Run {
            text: words.to_owned(),
            token: Token::Dim,
        },
    ]
}

/// The top border: the pane's name, and nothing else — the picker has no query
/// to state.
fn top(width: usize, charset: Charset) -> Row {
    frame(
        Glyph::TopLeft,
        Glyph::TopRight,
        vec![Run {
            text: TITLE.to_owned(),
            token: Token::Accent,
        }],
        width,
        charset,
    )
}

/// The bottom border: the keys the pane reads (`02` §8).
fn bottom(width: usize, charset: Charset) -> Row {
    frame(
        Glyph::BottomLeft,
        Glyph::BottomRight,
        vec![Run {
            text: keys(charset),
            token: Token::Dim,
        }],
        width,
        charset,
    )
}

/// The row `mode` sits on.
fn row_of(mode: PermissionMode) -> usize {
    MODES
        .iter()
        .position(|(known, _)| *known == mode)
        .unwrap_or_default()
}

/// The widest mode's name, so the words line up down the pane.
fn name_column() -> usize {
    MODES
        .iter()
        .map(|(mode, _)| cell_width(name(*mode)))
        .max()
        .unwrap_or(0)
}
