//! The session journal: an append-only `journal.ndjson` in the session's
//! state directory (`sessions/<id>/`), one event per line. It is the audit
//! record of what the user consented to — never a grant source: nothing
//! reads it back into a grant store, so a resumed session starts empty.

/// The journal file's name inside a session state directory. Reserved since
/// U1 (`session_paths.rs` in `saya-cli`); written from U7 on.
pub const JOURNAL_FILE: &str = "journal.ndjson";

/// Maximum journal bytes retained per session. Appends fail before action when
/// this ceiling would be crossed; an oversized existing journal fails closed.
pub const MAX_JOURNAL_BYTES: usize = 1 << 20;

mod events;
mod files;
mod journal;

pub use events::{BypassSource, GrantSource, JournalEvent};
pub use journal::SessionJournal;

#[cfg(test)]
#[path = "session_journal_tests.rs"]
mod tests;
