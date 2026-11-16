//! The journal's byte bound: appends past it are refused, oversized journals
//! fail closed on read, and one oversized event writes nothing.

use super::state_dir;
use crate::StoreError;
use crate::session_journal::{GrantSource, JOURNAL_FILE, MAX_JOURNAL_BYTES, SessionJournal};

#[test]
fn appending_past_the_journal_bound_is_refused() {
    let dir = state_dir("bound");
    let path = dir.join(JOURNAL_FILE);
    let mut existing = vec![b'x'; MAX_JOURNAL_BYTES];
    *existing.last_mut().expect("the bound is non-zero") = b'\n';
    std::fs::write(&path, existing).expect("bounded fixture writes");
    let journal = SessionJournal::open(&dir);
    assert_eq!(
        journal.granted("sql:analytics", GrantSource::Prompt),
        Err(StoreError::LimitExceeded),
        "an append that would exceed the journal bound is refused"
    );
    assert_eq!(
        std::fs::metadata(path).expect("journal remains").len() as usize,
        MAX_JOURNAL_BYTES
    );
}

#[test]
fn reading_an_oversized_journal_fails_closed() {
    let dir = state_dir("read_bound");
    std::fs::write(
        dir.join(JOURNAL_FILE),
        vec![b'{'; MAX_JOURNAL_BYTES.saturating_add(1)],
    )
    .expect("oversized fixture writes");
    assert_eq!(
        SessionJournal::open(&dir).read(),
        Err(StoreError::LimitExceeded),
        "journal reads never allocate beyond the persistence bound"
    );
}

#[test]
fn one_oversized_event_is_refused_without_writing() {
    let dir = state_dir("event_bound");
    let journal = SessionJournal::open(&dir);
    let token = "x".repeat(MAX_JOURNAL_BYTES);
    assert_eq!(
        journal.granted(&token, GrantSource::Prompt),
        Err(StoreError::LimitExceeded)
    );
    assert!(
        !dir.join(JOURNAL_FILE).exists(),
        "a refused event creates no file"
    );
}
