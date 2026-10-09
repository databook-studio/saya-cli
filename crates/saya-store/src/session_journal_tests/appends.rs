//! What appending does: lines accumulate in write order, a secret-shaped
//! token is redacted at the seam, and the file is born 0600.

use super::state_dir;
use crate::session_journal::{GrantSource, JOURNAL_FILE, JournalEvent, SessionJournal};

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
