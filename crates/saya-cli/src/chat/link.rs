//! The session's link to its run, and what a saved record's link means when the
//! session is resumed.
//!
//! A session outlives the run behind it — a run is one epoch of one job, and a
//! session rolls over to the next epoch, or to a new job — so the record carries
//! its link: which job the session belongs to, and the epoch the last save was
//! written at. The link is lineage only: it grants nothing, and it says where to
//! look.
//!
//! Resuming reads that link and the facts this process holds about the job, and
//! decides one of four things: attach the run this process already has open,
//! prepare a new epoch seeded with the transcript, open read-only with a reason,
//! or refuse a job that cannot be resumed at all.

use saya_store::{RedactedSession, RunLink};
use saya_types::JobId;

use super::transcript::{SettledTurn, Transcript};

/// Why a record that carries no link opens read-only: it was saved before the
/// session kept one, so there is no run to look for.
const NO_LINK: &str = "this record carries no run link: the transcript, mode and model are restored, and \
                       the next message prepares a run";

/// Why a record whose job folder is gone opens read-only.
const NO_FOLDER: &str = "the job's folder is gone: this session opens read-only, and the next message \
                         prepares a new job";

/// Why a record whose run another process holds opens read-only: two runs must
/// never number one job's events, and this process cannot attach to it.
const ELSEWHERE: &str = "another process holds the job's run open: this session opens read-only, and the \
                         next message prepares a run of its own";

/// The core's own words for a job this release takes no further turn on. The
/// refusal reuses them so the two surfaces state one sentence, not two.
const MANAGED_CHECKOUT: &str = "works in a managed checkout, which runs one turn per run; a further turn \
                                needs an in-place job";

/// What this process knows about the job a record links to when the session
/// resumes: no store read and no run is opened to learn any of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct RunFacts {
    /// Whether this very process holds the job's run open.
    pub(crate) open_in_this_process: bool,
    /// Whether the job's run finished.
    pub(crate) finished: bool,
    /// Whether the job's folder is gone from disk.
    pub(crate) job_dir_missing: bool,
    /// Whether the job works in a managed checkout rather than in place.
    pub(crate) managed_checkout: bool,
}

/// What a session does with the run a saved record names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Resume {
    /// The run is open in this process: the session attaches to it, at the epoch
    /// the link records.
    Attach {
        /// The job whose run is attached.
        job: JobId,
        /// The epoch of that run.
        epoch: u64,
    },
    /// The run is over: the next message prepares a new epoch seeded with the
    /// transcript.
    PrepareNewEpoch {
        /// The settled turns to seed it with, oldest first.
        seed: Vec<SettledTurn>,
    },
    /// The session opens read-only, with the one line that says why.
    ReadOnly {
        /// The sentence the front shows.
        reason: String,
    },
    /// The job cannot be resumed at all. The caller states it as the core's
    /// not-resumable refusal, whose own shape is the job and this detail.
    Refused {
        /// The job that cannot be resumed.
        job: JobId,
        /// Why, in the core's words.
        detail: String,
    },
}

/// Writes the link to the job and epoch a run was just prepared for: from now on
/// the record says which run the session belongs to.
pub(crate) fn after_prepare(record: &mut RedactedSession, job: &JobId, epoch: u64) {
    linked(record, job, epoch);
}

/// Replaces the link after a rollover: the same job's next epoch, or — when the
/// rollover starts a new job — the new job at its first epoch.
pub(crate) fn after_rollover(record: &mut RedactedSession, job: &JobId, epoch: u64) {
    linked(record, job, epoch);
}

/// What the session does with the run its record names.
///
/// A managed checkout is refused before anything else is looked at: which turn a
/// checkout-based job may take is a property of the job, not of this process's
/// view of it. Of the remaining rows, a folder that is gone cannot be attached or
/// re-prepared, a run this process holds is attached at its own epoch, and a
/// finished run gives way to a new epoch. The remaining case — a link this
/// process neither holds open nor sees finished — is a run another process owns,
/// which is read-only here rather than a second writer on one job.
pub(crate) fn decide(transcript: &Transcript, facts: RunFacts) -> Resume {
    let Some(link) = transcript.record().run.as_ref() else {
        return Resume::ReadOnly {
            reason: NO_LINK.to_owned(),
        };
    };
    if facts.managed_checkout {
        return Resume::Refused {
            job: link.job.clone(),
            detail: MANAGED_CHECKOUT.to_owned(),
        };
    }
    if facts.job_dir_missing {
        return Resume::ReadOnly {
            reason: NO_FOLDER.to_owned(),
        };
    }
    if facts.open_in_this_process {
        return Resume::Attach {
            job: link.job.clone(),
            epoch: link.epoch,
        };
    }
    if facts.finished {
        return Resume::PrepareNewEpoch {
            seed: transcript.turns(),
        };
    }
    Resume::ReadOnly {
        reason: ELSEWHERE.to_owned(),
    }
}

fn linked(record: &mut RedactedSession, job: &JobId, epoch: u64) {
    record.run = Some(RunLink {
        job: job.clone(),
        epoch,
    });
}

#[cfg(test)]
#[path = "link_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "resume_tests.rs"]
mod resume_tests;
