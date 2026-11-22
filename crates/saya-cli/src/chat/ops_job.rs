//! The job controls' one-release notes, and how often each is said.
//!
//! `/approve`, `/deny`, `/message`, `/pause`, `/continue <id> <agent>`,
//! `/cancel-child` and `/follow-up` are `/job`'s verbs under a deprecated
//! spelling for one release (`ref/03-commands.md` row 171). Each says its note
//! **once per session** — the `/status` aliases' rule (`chat::ops_status_notes`) —
//! so a session that runs `/approve` every turn is told once, not every time.
//!
//! In-memory only: carried on the session behind `#[serde(skip)]`, so a resumed
//! session starts with every note owed — a person who resumes has not read this
//! process's answers.

use crate::slash::{JobAlias, JobRequest};

/// Which job-alias notes this session has already said.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct JobNotes {
    said: [bool; JobAlias::ALL.len()],
}

impl JobNotes {
    /// Whether `alias`'s note is owed now, marking it said: true on the first
    /// call for that alias in this session, false afterwards.
    pub(crate) fn first_time(&mut self, alias: JobAlias) -> bool {
        let said = &mut self.said[alias.index()];
        let owed = !*said;
        *said = true;
        owed
    }

    /// The aliases whose notes this session has said, as the spellings a test
    /// or a surface reads back.
    pub(crate) fn said(&self) -> Vec<&'static str> {
        JobAlias::ALL
            .into_iter()
            .filter(|alias| self.said[alias.index()])
            .map(JobAlias::name)
            .collect()
    }
}

/// `request` with its alias note cleared unless this session still owes it.
///
/// The one way a surface says the note: it asks here, and the request that
/// reaches the job carries the note only the first time the alias is used.
pub(crate) fn claim_job_note(notes: &mut JobNotes, request: &mut JobRequest) {
    request.via_alias = request.via_alias.filter(|alias| notes.first_time(*alias));
}

#[cfg(test)]
#[path = "ops_job_tests.rs"]
mod tests;
