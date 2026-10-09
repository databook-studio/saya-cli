//! What reading does: an absent journal reads empty, a torn trailing line is
//! ignored and healed at the next open, a complete line that fails to parse
//! fails closed.

use super::state_dir;
use crate::StoreError;
use crate::session_journal::{GrantSource, JOURNAL_FILE, JournalEvent, SessionJournal};

/// An absent journal reads as an empty one — a session that never granted
/// has nothing to say, and reading says so rather than failing.
#[test]
fn an_absent_journal_reads_as_empty() {
    let dir = state_dir("absent");
    let journal = SessionJournal::open(&dir);
    assert_eq!(journal.read().expect("absent is empty"), Vec::new());
    assert!(!dir.join(JOURNAL_FILE).exists(), "reading creates nothing");
}

/// A torn trailing line — a crash mid-append — was never a complete event
/// and is ignored on read; the events before it read whole. And it is healed
/// at the next open (under the single-writer lock), so the next write cannot
/// concatenate onto it and turn a journal that reads into one that corrupts.
#[test]
fn a_torn_trailing_line_is_ignored_on_read_and_healed_at_open() {
    let dir = state_dir("torn");
    let journal = SessionJournal::open(&dir);
    journal
        .append(&JournalEvent::Granted {
            token: "sql:analytics".to_owned(),
            source: GrantSource::Seed,
        })
        .expect("first append");
    // Simulate a crash mid-append: half of a second line, no newline.
    let path = dir.join(JOURNAL_FILE);
    let mut raw = std::fs::read_to_string(&path).expect("read");
    raw.push_str("{\"event\":\"session-granted\",\"tok");
    std::fs::write(&path, raw).expect("torn tail written");
    assert_eq!(
        journal.read().expect("torn tail tolerated"),
        vec![JournalEvent::Granted {
            token: "sql:analytics".to_owned(),
            source: GrantSource::Seed,
        }],
        "the complete line reads whole; the torn one is ignored"
    );
    // The next process claims the session (the lock), opens the journal —
    // the torn tail is dropped — and its append lands as a whole line.
    let resumed = SessionJournal::open(&dir);
    resumed
        .granted("runner:bench", GrantSource::Prompt)
        .expect("post-crash append");
    let events = resumed.read().expect("read after append");
    assert_eq!(
        events.len(),
        2,
        "the post-crash event is readable: {events:?}"
    );
}

/// A complete line that fails to parse is corruption, not emptiness — the
/// journal fails closed rather than reading a record that dropped a line.
#[test]
fn a_complete_line_that_fails_to_parse_fails_closed() {
    let dir = state_dir("corrupt");
    let journal = SessionJournal::open(&dir);
    journal
        .append(&JournalEvent::Granted {
            token: "sql:analytics".to_owned(),
            source: GrantSource::Seed,
        })
        .expect("first append");
    let path = dir.join(JOURNAL_FILE);
    let raw = std::fs::read_to_string(&path).expect("read");
    std::fs::write(&path, format!("{raw}not-json\n")).expect("corrupt line written");
    assert_eq!(
        journal.read(),
        Err(StoreError::Invalid),
        "a complete corrupt line fails closed"
    );
}
