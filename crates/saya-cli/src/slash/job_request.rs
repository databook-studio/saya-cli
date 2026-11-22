//! The job surface's one-release spellings: `/approve`, `/deny`, `/message`,
//! `/pause`, `/continue <id> <agent>`, `/cancel-child` and `/follow-up` are
//! `/job`'s own verbs, still accepted at top level for one release
//! (`ref/03-commands.md` row 171, overlap O4).
//!
//! An alias is not a second command. Each of them parses into **the same**
//! `CodingCommand` its `/job <verb>` spelling parses, and the one session path
//! runs it; the only difference is the note the deprecated spelling states,
//! once per session (`crate::chat::ops_job`).
//!
//! Bare `/continue` is not here. It continues a paused turn — a turn of the
//! session's own run, never a job control — so it is its own variant,
//! `SlashCommand::ContinueTurn`. The argument spelling `/continue <id> <agent>`
//! is [`JobAlias::Continue`].

use crate::cli::CodingCommand;

/// Another spelling of a `/job` verb, and the one-line note it states once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobAlias {
    FollowUp,
    Approve,
    Deny,
    Message,
    Pause,
    Continue,
    CancelChild,
}

impl JobAlias {
    /// Every alias, in the order the notes are recorded.
    pub(crate) const ALL: [JobAlias; 7] = [
        JobAlias::FollowUp,
        JobAlias::Approve,
        JobAlias::Deny,
        JobAlias::Message,
        JobAlias::Pause,
        JobAlias::Continue,
        JobAlias::CancelChild,
    ];

    /// The spelling, without its `/`.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::FollowUp => "follow-up",
            Self::Approve => "approve",
            Self::Deny => "deny",
            Self::Message => "message",
            Self::Pause => "pause",
            Self::Continue => "continue",
            Self::CancelChild => "cancel-child",
        }
    }

    /// The one-line note: one operation, one replacement, one release. The
    /// child controls' argument forms are named, so a reader of the note can
    /// type the replacement without `/help`.
    pub(crate) const fn text(self) -> &'static str {
        match self {
            Self::FollowUp => {
                "note: /follow-up is now /job follow-up — this release is the last to answer it"
            }
            Self::Approve => {
                "note: /approve is now /job approve — this release is the last to answer it"
            }
            Self::Deny => "note: /deny is now /job deny — this release is the last to answer it",
            Self::Message => {
                "note: /message is now /job message — this release is the last to answer it"
            }
            Self::Pause => "note: /pause is now /job pause — this release is the last to answer it",
            Self::Continue => {
                "note: /continue <id> <agent> is now /job continue <id> <agent> — this release is \
                 the last to answer it; bare /continue continues a paused turn"
            }
            Self::CancelChild => {
                "note: /cancel-child is now /job cancel-child — this release is the last to \
                 answer it"
            }
        }
    }

    /// Where this alias sits in [`JobAlias::ALL`], which is where the session
    /// records that its note has been said.
    pub(crate) const fn index(self) -> usize {
        match self {
            Self::FollowUp => 0,
            Self::Approve => 1,
            Self::Deny => 2,
            Self::Message => 3,
            Self::Pause => 4,
            Self::Continue => 5,
            Self::CancelChild => 6,
        }
    }
}

/// The one operation a job spelling names, plus the top-level spelling that
/// stated it while that spelling's note is still owed.
///
/// `/job <verb>`, `/start` and `/follow-up`'s `/job` replacement produce this
/// with no alias; the seven deprecated spellings produce it with their alias,
/// and the session clears the alias once the note has been said — so the note
/// rides the operation rather than a second command's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRequest {
    /// The operation, identical for both spellings of it.
    pub command: CodingCommand,
    /// The deprecated spelling that stated this, while its note is owed.
    pub via_alias: Option<JobAlias>,
}

impl JobRequest {
    /// A canonical spelling: `/job <verb>`, `/start`.
    pub(crate) const fn job(command: CodingCommand) -> Self {
        Self {
            command,
            via_alias: None,
        }
    }

    /// A deprecated top-level spelling, and the alias it is.
    pub(crate) const fn alias(command: CodingCommand, alias: JobAlias) -> Self {
        Self {
            command,
            via_alias: Some(alias),
        }
    }

    /// The note this request still owes, if any.
    pub(crate) fn note(&self) -> Option<&'static str> {
        self.via_alias.map(JobAlias::text)
    }
}
