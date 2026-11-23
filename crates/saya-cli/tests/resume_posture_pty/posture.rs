//! The host posture a `--resume` launch derives: the real binary, on a pty,
//! with a recorded session whose pin binds a root. The piped REPL never prints
//! a panel (it is TTY-gated), so the pty surface is the observable one: after
//! the resume the session must still state the route the resumed universe
//! composed (`Host lane: composed`) and the deny list the user-layer config
//! composed into it (`Deny list: curl`) — facts that ride `SessionState` and are
//! derived on a resume as well as on a fresh start.
//!
//! Both facts used to ride the status row (`host unsandboxed`, `deny: curl`).
//! UI4 cleared that row to the mode, the model and the database, so they are
//! read where they live now, on the commands that state the session's
//! universe: `/status`'s Workspace section states the host posture, and the
//! `/permissions` view states the deny list beside it.

use crate::support::{RESUMED_ID, fixture, launch};

#[test]
fn a_resumed_launch_paints_the_host_posture_the_universe_composed() {
    let home = fixture("resume");
    let mut session = launch(
        &home,
        &["--approval-mode", "read-only", "--resume", RESUMED_ID],
    );

    // The surface is up before a line is typed into it.
    session.wait_for("? help");
    // The resumed universe composed the route (the pin re-binds) and `/status`
    // states it: the Workspace section's own line.
    session.send_line("/status");
    session.wait_for("Host lane: composed");
    // The deny list the same universe composed, in the view that names it.
    session.send_line("/permissions");
    session.wait_for("Deny list: curl");
}
