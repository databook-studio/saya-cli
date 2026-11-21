//! The words a mode change says: the pane's title and key row, the one line
//! each mode means (`02` §8), the compact line the transcript keeps (§3.10) and
//! the toast that names what the mode does (§4, J6).
//!
//! One table per kind of sentence, read by the picker's rows, by the update's
//! change and by any test that wants to know what a mode says — so a mode
//! cannot be called one thing in the pane and another in the status line.

use saya_agent::PermissionMode;

use super::palette_frame::sep;
use crate::front::tui::theme::Charset;
use crate::interactive::session::permissions::mode::name;

/// The pane's title, on its top border.
pub(crate) const TITLE: &str = "Mode";

/// The six modes of `02` §8, in the order it lists them, each with the one
/// line it means — the design's words, verbatim.
pub(crate) const MODES: [(PermissionMode, &str); 6] = [
    (
        PermissionMode::Ask,
        "every edit, command, fetch and query asks first",
    ),
    (
        PermissionMode::AcceptEdits,
        "file edits run; commands, fetches, queries ask",
    ),
    (PermissionMode::Plan, "read and plan only; writes refuse"),
    (
        PermissionMode::Bypass,
        "nothing asks (allowed by [permissions])",
    ),
    (
        PermissionMode::ReadOnly,
        "no writes, no host commands; nothing asks",
    ),
    (
        PermissionMode::Never,
        "no tool needs approval because none are offered",
    ),
];

/// The arrows the key row names. `02` §11's table carries the transcript's
/// glyphs, so the pair this pane draws lives here, as the palette's `↵` does.
const ARROWS: &str = "↑↓";
const ARROWS_ASCII: &str = "Up/Down";

/// The one line `mode` means.
pub(crate) fn words(mode: PermissionMode) -> &'static str {
    MODES
        .iter()
        .find(|(known, _)| *known == mode)
        .map_or("", |(_, words)| words)
}

/// The key row the pane draws, in the glyph set in force.
pub(crate) fn keys(charset: Charset) -> String {
    let mark = sep(charset);
    format!(
        "{} choose {mark} Enter set {mark} Esc cancel",
        arrows(charset)
    )
}

/// The one compact line a change puts in the transcript (`02` §3.10, J6).
pub(crate) fn line(mode: PermissionMode) -> String {
    format!("mode: {}", name(mode))
}

/// The toast a change says (`02` §4): the mode, and what it means in one
/// sentence. `plan`'s sentence is the design's own; the others follow its
/// shape with the picker's words, which is what J6 asks a toast to say.
pub(crate) fn toast(mode: PermissionMode) -> String {
    format!("mode → {}: {}", name(mode), meaning(mode))
}

/// The arrows, in the glyph set in force.
const fn arrows(charset: Charset) -> &'static str {
    match charset {
        Charset::Utf8 => ARROWS,
        Charset::Ascii => ARROWS_ASCII,
    }
}

/// What `mode` means, in the one sentence a toast has room for.
const fn meaning(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Ask => "every edit, command, fetch and query asks first",
        PermissionMode::AcceptEdits => "file edits run; commands, fetches and queries still ask",
        PermissionMode::Plan => "write tools refuse until you leave plan",
        PermissionMode::Bypass => "nothing asks — allowed by [permissions]",
        PermissionMode::ReadOnly => "no writes and no host commands; nothing asks",
        PermissionMode::Never => "no tool is offered, so nothing asks",
    }
}
