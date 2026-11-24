// Unix-only: the pty harness behind `tui_pty_smoke.rs` is macOS/Linux, and
// the same ConPTY unknown applies here.
#![cfg(not(windows))]

//! The saved-session picker, on a pty, over the real binary.
//!
//! What the earlier cases cannot prove: that `/sessions` paints its pane on a
//! real terminal, that `↵` on a row reaches the run the row names, and that the
//! screen afterwards says what it resumed — the conversation's own words from
//! the record, in the transcript, with the status row naming the session it is
//! on.

#[path = "tui_sessions_pty/support.rs"]
mod support;

#[path = "tui_sessions_pty/picker.rs"]
mod picker;
