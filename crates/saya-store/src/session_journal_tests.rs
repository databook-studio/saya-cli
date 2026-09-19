//! The session journal's properties: one line per event in the settled
//! shape, redacted at write through the repo's own redaction seam, append
//! only, 0600, torn-tail tolerant, corrupt-line fail-closed.

use super::{
    BypassSource, GrantSource, JOURNAL_FILE, JournalEvent, MAX_JOURNAL_BYTES, SessionJournal,
};
use crate::StoreError;
use std::path::PathBuf;

/// A fresh state directory per test, the way a session's is created (0700).
fn state_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("saya-journal-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("state dir creates");
    dir
}

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

/// The journal is append-only: each event adds one line, in write order, and
/// nothing written earlier is rewritten or removed.
#[test]
fn appends_accumulate_in_write_order_without_rewriting() {
    let dir = state_dir("append");
    let journal = SessionJournal::open(&dir);
    journal
        .append(&JournalEvent::Granted {
            token: "sql:analytics".to_owned(),
            source: GrantSource::Seed,
        })
        .expect("first append");
    let first = std::fs::read_to_string(dir.join(JOURNAL_FILE)).expect("read");
    journal
        .append(&JournalEvent::Granted {
            token: "runner:bench".to_owned(),
            source: GrantSource::Prompt,
        })
        .expect("second append");
    let both = std::fs::read_to_string(dir.join(JOURNAL_FILE)).expect("read");
    assert!(
        both.starts_with(&first),
        "the second append never rewrites the first line: {both}"
    );
    assert_eq!(
        journal.read().expect("the journal parses"),
        vec![
            JournalEvent::Granted {
                token: "sql:analytics".to_owned(),
                source: GrantSource::Seed,
            },
            JournalEvent::Granted {
                token: "runner:bench".to_owned(),
                source: GrantSource::Prompt,
            },
        ],
        "read returns every event in write order"
    );
}

/// The journal must not be a place secrets land: a token whose text would
/// trip redaction is written redacted, through the repo's existing redaction
/// seam — the same one the session file and the run journal use. The write
/// goes through the same `granted` seam production calls.
#[test]
fn a_token_carrying_secret_shaped_text_is_written_redacted() {
    let dir = state_dir("redact");
    let journal = SessionJournal::open(&dir);
    journal
        .granted("sql:analytics&password=hunter2", GrantSource::Prompt)
        .expect("the append lands");
    let raw = std::fs::read_to_string(dir.join(JOURNAL_FILE)).expect("read");
    assert!(
        !raw.contains("hunter2"),
        "the secret reached the journal raw: {raw}"
    );
    assert!(
        raw.contains("[redacted]"),
        "the credential shape was redacted, not dropped: {raw}"
    );
    // The redacted bytes stay valid NDJSON the journal can read back.
    assert_eq!(
        journal.read().expect("redacted line still parses"),
        vec![JournalEvent::Granted {
            token: "sql:analytics&password=[redacted]".to_owned(),
            source: GrantSource::Prompt,
        }],
        "the round-trip carries the redacted token"
    );
}

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

/// The journal is written 0600: a session state directory's bytes are the
/// session's alone, and the file must not rely on the directory mode alone
/// (the scratch database's own rule).
#[cfg(unix)]
#[test]
fn the_journal_file_is_created_0600() {
    use std::os::unix::fs::PermissionsExt;
    let dir = state_dir("mode");
    let journal = SessionJournal::open(&dir);
    journal
        .append(&JournalEvent::Granted {
            token: "sql:analytics".to_owned(),
            source: GrantSource::Seed,
        })
        .expect("append creates the file");
    let mode = std::fs::metadata(dir.join(JOURNAL_FILE))
        .expect("the file exists")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600, "the journal is 0600");
}

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
