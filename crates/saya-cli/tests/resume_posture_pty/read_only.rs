//! `saya --resume <id>` on a pty opens the NEW TUI, and the resumed record's own
//! state is on it.
//!
//! The fixture is R1's 0.5.0 read-only record: a session saved with no run link
//! (`ref/REPORT-R1.md`, the "no link" row). Its link decision is `ReadOnly`, so
//! the front restores the transcript and prepares a run of its own on the next
//! message — and the surface states that about itself in the composer's own row
//! (`02` §7, `view::composer::State::ReadOnlyRecord`), where the old TUI had no
//! such state to paint.
//!
//! Two assertions, in the order the surface states them:
//!
//! 1. the NEW surface is the one that opened — its status line's idle word is
//!    `? help`, where the old surface said `? for help`;
//! 2. the record opened read-only, and the composer says what the next message
//!    does.

use crate::support::{RESUMED_ID, fixture, launch};

/// `view::composer`'s placeholder for a read-only record, whole: the surface's
/// own sentence about the state the resume decision put this session in.
const READ_ONLY_PLACEHOLDER: &str = "This is a 0.5.0 session — your next message starts a new run";

#[test]
fn a_resumed_read_only_record_opens_the_new_surface_with_its_own_row() {
    let home = fixture("read-only");
    let mut session = launch(
        &home,
        &["--approval-mode", "read-only", "--resume", RESUMED_ID],
    );

    // The surface is the new one, not the TUI this release replaces: the status
    // line's idle word is its own.
    session.wait_for("? help");
    // The resumed record's state shows: no run link, so the record opens
    // read-only, and the composer states the record's row rather than an idle
    // prompt.
    let screen = session.wait_for(READ_ONLY_PLACEHOLDER);
    assert!(
        !screen.contains("? for help"),
        "the old surface's status word is gone: the launch opened the new one\n{screen}"
    );
}
