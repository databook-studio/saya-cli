//! The host posture a `--resume` launch derives: the real binary, on a pty,
//! with a recorded session whose pin binds a root. The piped REPL never prints
//! the status line (it is TTY-gated), so the pty surface is the observable
//! header: the status line must carry the route the resumed universe composed
//! (`host unsandboxed`) and the deny list the user-layer config composed into it
//! (`deny: curl`) — facts that ride `SessionState` and are derived on a resume as
//! well as on a fresh start.
//!
//! The words are the new surface's (T19 switched the launch over):
//! `view::status_segments` states both as caveat segments on the one status
//! line, where the old status bar said `Host commands unsandboxed` and
//! `Denied: curl` on its own context line.

use crate::support::{RESUMED_ID, fixture, launch};

#[test]
fn a_resumed_launch_paints_the_host_posture_the_universe_composed() {
    let home = fixture("resume");
    let mut session = launch(
        &home,
        &["--approval-mode", "read-only", "--resume", RESUMED_ID],
    );

    // The status line's posture facts, from the state the launch derived: the
    // resumed universe composed the route (the pin re-binds), and the config
    // deny list composed into it. The workspace segment yields to fit on a
    // narrow row; the posture segments never do.
    session.wait_for("host unsandboxed");
    session.wait_for("deny: curl");
}
