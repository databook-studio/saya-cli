//! `/sessions` → `↵` on a real terminal: the pane paints the saved sessions,
//! choosing one resumes it, and the screen afterwards is that conversation.

use super::support::{NEWER, fixture, launch};

/// The pane's title, as its top border paints it (`view::sessions::TITLE`).
const PANE: &str = "Sessions";

/// Opening the picker lists both saved sessions; `↵` resumes the newest one,
/// and the screen says so in the record's own words.
#[test]
fn choosing_a_saved_session_from_the_picker_resumes_it() {
    let home = fixture("picker");
    let mut session = launch(&home);
    session.wait_for("saya");

    session.send_line("/sessions");
    let screen = session.wait_for(PANE);
    assert!(
        screen.contains(NEWER) || screen.contains("the newer question"),
        "the pane lists the saved sessions:\n{screen}"
    );

    // `↵` on the row: the pane closes and the surface says what it resumed.
    session.press(b"\r");
    let screen = session.wait_for("resumed ");
    assert!(
        screen.contains("resumed the newer question"),
        "the surface says which session it resumed:\n{screen}"
    );
    assert!(
        screen.contains("the newer question") && screen.contains("newer answer"),
        "the transcript is the resumed conversation:\n{screen}"
    );
    assert!(
        !screen.contains(PANE),
        "the picker closed on the row it chose:\n{screen}"
    );

    // Esc on the composer is the surface's own; the session is still the
    // resumed one, and the run is still open on nothing the pane covers.
    session.press(b"\x1b");
    let screen = session.wait_for("resumed the newer question");
    assert!(
        screen.contains("resumed the newer question"),
        "the resumed session stays the one the surface is on:\n{screen}"
    );
}

/// Esc closes the picker and resumes nothing.
#[test]
fn escape_closes_the_picker_without_resuming() {
    let home = fixture("escape");
    let mut session = launch(&home);
    session.wait_for("saya");

    session.send_line("/sessions");
    session.wait_for(PANE);

    session.press(b"\x1b");
    let screen = session.wait_for("saya");
    assert!(
        !screen.contains("resumed the newer question"),
        "nothing was resumed:\n{screen}"
    );
}
