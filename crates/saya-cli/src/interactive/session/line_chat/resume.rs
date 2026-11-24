//! What a resumed record's link means for the line front.
//!
//! The link and this process's facts about the job are A6's decision
//! ([`crate::chat::decide`]); this module states the facts the front can prove
//! — the job's own records on disk, and whether the job recorded a terminal
//! state — and reads the decision back as what the front does next.
//!
//! A resumed process holds no run of its own, so the run a link names is one
//! the front can still take: the job's records are here, the run recorded no
//! terminal state, and the job works in place. That is the *attach* row, and
//! the next line runs on the run the record named rather than on a job this
//! process would otherwise mint. A finished run prepares that same job's next
//! epoch; a record with no link, or one whose folder is gone, opens read-only
//! and its next line prepares a run of its own; a managed checkout is refused
//! outright.
//!
//! **One decision, one place.** The front consults this when it opens the
//! session's run, and every surface of it — the read-only sentence, the
//! refusal, the epoch that follows — is read from the same answer.

use std::path::Path;

use saya_store::RedactedSession;
use saya_types::JobId;

use crate::chat::{Resume, RunFacts, SettledTurn, Transcript, decide};
use crate::commands::coding::job_records;

/// What the front does with the run a resumed record names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Opening {
    /// The linked run is still open: the next line is a turn of it, at the
    /// epoch the engine hands back.
    Attach {
        /// The job the record links.
        job: JobId,
    },
    /// The linked run is over: the next line is a turn of the same job's next
    /// epoch, seeded with the kept tail.
    NewEpoch {
        /// The job the record links.
        job: JobId,
        /// The settled turns it is seeded with, oldest first.
        seed: Vec<SettledTurn>,
    },
    /// The transcript is shown and the record opens read-only, with the one
    /// line that says why; the next message prepares a run of its own.
    ReadOnly {
        /// The sentence the front shows.
        reason: String,
    },
    /// The job cannot be resumed at all: one sentence, and the process leaves.
    Refused {
        /// The sentence the front shows.
        sentence: String,
    },
}

/// The facts about a linked job's records that this process can read without
/// opening a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Facts {
    /// Whether the job recorded a terminal state, as the engine's own record
    /// states it.
    pub(crate) finished: bool,
    /// Whether the job's folder is gone from disk.
    pub(crate) job_dir_missing: bool,
    /// Whether the job works in a managed checkout rather than in place.
    pub(crate) managed_checkout: bool,
}

/// The facts `state_root` holds about `job`, given whether the engine thinks it
/// finished: a folder that is not there answers all three, and a job that
/// recorded no in-place root works in a managed checkout.
pub(crate) fn facts(state_root: &Path, job: &JobId, finished: bool) -> Facts {
    let present = job_records::present(state_root, job);
    Facts {
        finished,
        job_dir_missing: !present,
        managed_checkout: present && !job_records::works_in_place(state_root, job),
    }
}

/// What the front does with the run `record` names, over `facts`.
///
/// The line front holds no run of the link's job, so the one fact A6's table
/// asks that only a surface can answer — whether this process already drives
/// the run — is what a *usable* run means here: the job's records are present,
/// it recorded no terminal state, and it works in place. Anything else cannot
/// be attached to, and the table's own rows decide the rest.
pub(crate) fn opening(record: &RedactedSession, facts: Facts) -> Opening {
    let linked = record.run.as_ref().map(|link| link.job.clone());
    match decision(record, facts) {
        Resume::Attach { job, .. } => Opening::Attach { job },
        Resume::PrepareNewEpoch { seed } => Opening::NewEpoch {
            // `decide` reaches this row only for a record that carries a link:
            // the seed it states is that record's own conversation.
            job: linked.expect("a new epoch names the job the record links"),
            seed,
        },
        Resume::ReadOnly { reason } => Opening::ReadOnly { reason },
        Resume::Refused { job, detail } => Opening::Refused {
            sentence: format!("job {job} {detail}"),
        },
    }
}

/// A6's own decision for `record` over `facts`: the one answer this process's
/// fronts read — the run a session opens, and the mark a row of the sessions
/// picker carries (`crate::front::tui::sessions_picker`).
pub(crate) fn decision(record: &RedactedSession, facts: Facts) -> Resume {
    // A folded conversation is the one thing a run cannot carry: the turns a
    // compaction replaced are still in the job's own history, and continuing
    // that run would replay what the session has already forgotten. Such a
    // record opens read-only like any other link this process cannot take, and
    // its next line prepares a run seeded from the record alone.
    let folded = record.compaction_summary.is_some();
    let usable = !facts.finished && !facts.job_dir_missing && !facts.managed_checkout && !folded;
    decide(
        &Transcript::from_record(record.clone()),
        RunFacts {
            open_in_this_process: usable,
            finished: facts.finished,
            job_dir_missing: facts.job_dir_missing,
            managed_checkout: facts.managed_checkout,
        },
    )
}
