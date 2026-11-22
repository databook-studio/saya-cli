//! What deleting a saved session does with the run its record links.
//!
//! A session's record names the job its run is on (`RunLink`), and that job is
//! the session's own: a chat session opens one job and every epoch of its run
//! stays on it. So deleting the session is deleting the job — but only the job
//! the session opened, only once it is over, and never while a live process
//! still holds the folder it writes.
//!
//! The decision is a pure function of the link and of what the job's own records
//! state ([`JobFacts`]): the record cannot see a process, so a lease it reads as
//! held is the only liveness it may act on, and it acts by refusing everything.

use crate::commands::coding::job_records::JobFacts;
use saya_store::RunLink;
use saya_types::JobId;

/// The sentence a refused delete states: the session and the job it names are
/// left exactly as they were.
pub(crate) const RUN_STILL_OPEN: &str = "the session's run is still open — stop it first";

/// What a session's link means for the delete that found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LinkedRun {
    /// The record links no run: the session deletes as it always did.
    None,
    /// The record links a job that is not the session's run to delete. The
    /// session record still goes; the job stays.
    Keep {
        /// Why the job is kept, in one phrase.
        why: &'static str,
    },
    /// The record links the session's own finished run: the job goes with the
    /// record.
    Delete {
        /// The job to remove.
        job: JobId,
    },
    /// The job's run may still be live: nothing is deleted at all.
    Refused {
        /// The job the refusal names.
        job: JobId,
    },
}

/// Why a linked job stays: it was opened by the CLI, not by a session.
pub(crate) const OPENED_BY_THE_CLI: &str =
    "the job it links was opened from the command line, not by a session";

/// Why a linked job stays: the job recorded no terminal state.
pub(crate) const NOT_FINISHED: &str = "the job it links has not finished";

/// Why a linked job stays: there is no job directory left to remove.
pub(crate) const ALREADY_GONE: &str = "the job it links keeps no records here";

/// What to do with the job `link` names, given what that job's records state.
///
/// The order is the one the operator's data deserves. A job the session did
/// not open is not the session's run at all, so its folder lease says nothing
/// about this session and it is left where it is — even while some other run
/// holds it. Only then is liveness read: the session's own run holding the
/// folder's lease means the run may be writing the folder right now, so
/// nothing at all is deleted and the whole delete is refused. A run that has
/// not finished is kept for the same reason a live one is refused, minus the
/// refusal: it is not this delete's to remove. What is left is the session's
/// own finished run, which goes with the record.
pub(crate) fn decide(link: Option<&RunLink>, facts: JobFacts) -> LinkedRun {
    let Some(link) = link else {
        return LinkedRun::None;
    };
    if !facts.records_present {
        return LinkedRun::Keep { why: ALREADY_GONE };
    }
    if facts.origin.is_cli() {
        return LinkedRun::Keep {
            why: OPENED_BY_THE_CLI,
        };
    }
    if facts.lease_held {
        return LinkedRun::Refused {
            job: link.job.clone(),
        };
    }
    if !facts.terminal {
        return LinkedRun::Keep { why: NOT_FINISHED };
    }
    LinkedRun::Delete {
        job: link.job.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::coding::JobOrigin;

    fn facts(origin: JobOrigin, terminal: bool, lease_held: bool, present: bool) -> JobFacts {
        JobFacts {
            origin,
            terminal,
            lease_held,
            records_present: present,
        }
    }

    fn link() -> RunLink {
        RunLink {
            job: JobId::parse("job-one").expect("the fixture's job id parses"),
            epoch: 1,
        }
    }

    #[test]
    fn a_record_with_no_link_names_no_job() {
        assert_eq!(decide(None, JobFacts::default()), LinkedRun::None);
    }

    #[test]
    fn a_session_run_that_finished_is_deleted() {
        let decided = decide(Some(&link()), facts(JobOrigin::Session, true, false, true));
        assert_eq!(
            decided,
            LinkedRun::Delete { job: link().job },
            "{decided:?}"
        );
    }

    #[test]
    fn a_job_the_command_line_opened_is_kept_even_while_a_run_holds_it() {
        let decided = decide(Some(&link()), facts(JobOrigin::Cli, true, true, true));
        assert_eq!(
            decided,
            LinkedRun::Keep {
                why: OPENED_BY_THE_CLI
            }
        );
    }

    #[test]
    fn a_session_run_whose_lease_is_held_refuses_the_delete() {
        let decided = decide(Some(&link()), facts(JobOrigin::Session, true, true, true));
        assert_eq!(
            decided,
            LinkedRun::Refused { job: link().job },
            "a held run is refused whether or not it recorded an outcome"
        );
    }

    #[test]
    fn a_session_run_that_has_not_finished_is_kept() {
        let decided = decide(Some(&link()), facts(JobOrigin::Session, false, false, true));
        assert_eq!(decided, LinkedRun::Keep { why: NOT_FINISHED });
    }

    #[test]
    fn a_link_to_a_job_that_keeps_no_records_is_kept() {
        let decided = decide(
            Some(&link()),
            facts(JobOrigin::Session, false, false, false),
        );
        assert_eq!(decided, LinkedRun::Keep { why: ALREADY_GONE });
    }
}
