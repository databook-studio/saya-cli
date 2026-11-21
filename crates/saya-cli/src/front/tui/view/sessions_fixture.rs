//! Test support for the sessions picker: the store's own listing row, the words
//! a record adds to it, a resume decision per state, and the pane read back as
//! the words a terminal would show.
//!
//! The four rows of [`picker`] are the four states a listed session can be in:
//! the run this process holds, a finished run, a record with no run, and a
//! second finished run so a filter has a row to keep and a row to drop.

use super::palette_fixture::inside;
use super::sessions::Picker;
use super::sessions_row::{Facts, SessionRow};
use super::sessions_rows;
use crate::chat::Resume;
use crate::commands::saved_sessions::SessionRow as StoredRow;
use saya_store::SessionCapacityStatus;
use saya_types::JobId;

/// The moment the fixture's ages are measured from: every stamp below is that
/// much before it, so no test reads a clock.
pub(super) const NOW: u128 = 1_700_000_000_000;

/// The mark of a run this process holds.
pub(super) const OPEN: &str = "●";

/// The mark of a finished run.
pub(super) const DONE: &str = "✓";

/// The mark of a record with no run. The glyph table the theme holds does not
/// carry it, so the pane names it where it draws it.
pub(super) const RECORD: &str = "◌";

/// The store's own listing row for a record: what the listing hands the pane,
/// beside the words the record carries.
pub(super) fn stored(id: &str, modified_unix_ms: u128) -> StoredRow {
    StoredRow {
        id: id.to_owned(),
        size_bytes: 4096,
        status: SessionCapacityStatus::Ok,
        modified_unix_ms,
    }
}

/// The words a record adds to the store's row.
pub(super) fn facts(title: &str, database: Option<&str>, turns: usize) -> Facts {
    Facts {
        title: title.to_owned(),
        database: database.map(str::to_owned),
        turns,
    }
}

/// One listed row, built the way the runtime builds it: the store's row, the
/// record's words and the decision resuming it would take.
pub(super) fn listed(
    id: &str,
    title: &str,
    database: Option<&str>,
    turns: usize,
    modified_unix_ms: u128,
    decision: Resume,
    now_ms: u128,
) -> SessionRow {
    SessionRow::listed(
        &stored(id, modified_unix_ms),
        facts(title, database, turns),
        &decision,
        now_ms,
    )
}

/// The job the fixture's run links name.
fn job() -> JobId {
    JobId::parse("fix-pricing").expect("the fixture's job id parses")
}

/// The run is open in this process.
pub(super) fn attach() -> Resume {
    Resume::Attach {
        job: job(),
        epoch: 3,
    }
}

/// The run finished: resuming seeds a new epoch with the conversation.
pub(super) fn finished() -> Resume {
    Resume::PrepareNewEpoch { seed: Vec::new() }
}

/// The record carries no run this process can open.
pub(super) fn read_only() -> Resume {
    Resume::ReadOnly {
        reason: "this record carries no run link".to_owned(),
    }
}

/// The job cannot be resumed at all.
pub(super) fn refused() -> Resume {
    Resume::Refused {
        job: job(),
        detail: "works in a managed checkout, which runs one turn per run".to_owned(),
    }
}

/// The picker the tests read: one session of each state, newest first.
pub(super) fn picker() -> Picker {
    Picker::new(vec![
        listed(
            "sess-open",
            "fix the failing pricing test",
            None,
            11,
            NOW - 30_000,
            attach(),
            NOW,
        ),
        listed(
            "sess-orders",
            "how many orders did we get since December?",
            Some("demo"),
            6,
            NOW - 7_200_000,
            finished(),
            NOW,
        ),
        listed(
            "sess-revenue",
            "revenue by region (0.5.0)",
            Some("prod"),
            3,
            NOW - 1_036_800_000,
            read_only(),
            NOW,
        ),
        listed(
            "sess-dryrun",
            "add dry-run flag",
            None,
            4,
            NOW - 259_200_000,
            finished(),
            NOW,
        ),
    ])
}

/// The pane's rows at `width` x `height`, as their text.
pub(super) fn pane(picker: &Picker, width: usize, height: usize) -> Vec<String> {
    sessions_rows::rows(picker, width, height)
        .iter()
        .map(inside)
        .collect()
}
