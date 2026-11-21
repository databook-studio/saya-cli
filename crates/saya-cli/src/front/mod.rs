//! The new front end: the TUI that replaces `interactive::tui` one screen at a
//! time (`06` §5).
//!
//! T0 lands only the theme — semantic tokens, glyphs and the `--ascii` fallback
//! as pure data — so the screens that follow name meaning instead of hue. No
//! rendering, no widgets, no event loop: nothing here runs yet.

pub(crate) mod tui;
