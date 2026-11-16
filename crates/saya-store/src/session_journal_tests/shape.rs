//! The session journal's settled line shapes: one granted grant and one
//! bypass activation, byte-exact.

use super::state_dir;
use crate::session_journal::{
    BypassSource, GrantSource, JOURNAL_FILE, JournalEvent, SessionJournal,
};

/// One granted line is exactly the settled shape: internally tagged on
/// `event`, the token and source carried whole.
#[test]
fn a_granted_line_is_exactly_the_settled_shape() {
    let dir = state_dir("shape");
    let journal = SessionJournal::open(&dir);
    journal
        .append(&JournalEvent::Granted {
            token: "sql:analytics".to_owned(),
            source: GrantSource::Prompt,
        })
        .expect("the first append creates the journal");
    let raw = std::fs::read_to_string(dir.join(JOURNAL_FILE)).expect("the journal reads");
    assert_eq!(
        raw, "{\"event\":\"session-granted\",\"token\":\"sql:analytics\",\"source\":\"prompt\"}\n",
        "one line, byte-exact to the settled shape: {raw}"
    );
}

/// A bypass activation is its own event — not a token grant — so a reader
/// filtering grants never conflates the mode with a token. The shape mirrors
/// the grant line's: `event`, then the source that produced the consent.
#[test]
fn a_bypass_activation_line_is_its_own_event_shape() {
    let dir = state_dir("bypass");
    let journal = SessionJournal::open(&dir);
    journal
        .append(&JournalEvent::Bypass {
            source: BypassSource::Command,
        })
        .expect("the append lands");
    let raw = std::fs::read_to_string(dir.join(JOURNAL_FILE)).expect("the journal reads");
    assert_eq!(
        raw, "{\"event\":\"session-bypass\",\"source\":\"command\"}\n",
        "one line, byte-exact: {raw}"
    );
}
