//! The approval modes' vocabulary: one spelling per mode, parsed here and
//! carried everywhere else as the type.

use std::{fmt, str::FromStr};

/// Controls what the approval engine answers for a tool call. `ask` renders
/// a prompt per call; `accept-edits` asks for everything but the workspace
/// file family (`write`, `edit`), which runs unasked; `read-only`
/// auto-approves read-shaped tools only and denies the rest; `never` denies
/// everything; `bypass` allows every call without asking — the per-call
/// consent given once at launch, with every structural guard (the SQL safety
/// layer, the sandbox, the allowlists) untouched.
///
/// Only four of these are `--approval-mode` spellings, and
/// [`ApprovalPolicy::AcceptEdits`] is not one of them: it is a *session*
/// consent — a reader's, like a grant — so it is reached only through the
/// session's own permission-mode switch, never through a launch flag that
/// every surface (including a run with no reader) also accepts. The parse
/// below is that grammar's authority, so a spelling it refuses cannot be
/// smuggled in by a flag, a config file or a resumed session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ApprovalPolicy {
    #[default]
    Ask,
    AcceptEdits,
    ReadOnly,
    Never,
    Bypass,
}

impl ApprovalPolicy {
    /// The byte this posture occupies in a [`SessionPolicy`]'s mode cell.
    /// The encoding lives beside the variants so a new arm cannot be added
    /// without one, and it is the cell's whole vocabulary — nothing else
    /// writes it.
    ///
    /// [`SessionPolicy`]: super::contracts::SessionPolicy
    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::Ask => 0,
            Self::AcceptEdits => 1,
            Self::ReadOnly => 2,
            Self::Never => 3,
            Self::Bypass => 4,
        }
    }

    /// The posture a cell byte holds. Only [`code`](Self::code) writes these
    /// bytes, so the fallback is unreachable; it reads as the default posture
    /// rather than as any widening, so a cell that somehow held something
    /// else would ask rather than allow.
    pub(crate) const fn from_code(code: u8) -> Self {
        match code {
            1 => Self::AcceptEdits,
            2 => Self::ReadOnly,
            3 => Self::Never,
            4 => Self::Bypass,
            _ => Self::Ask,
        }
    }
}

impl FromStr for ApprovalPolicy {
    type Err = ApprovalPolicyParseError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "ask" => Ok(Self::Ask),
            "read-only" => Ok(Self::ReadOnly),
            "never" => Ok(Self::Never),
            "bypass" => Ok(Self::Bypass),
            _ => Err(ApprovalPolicyParseError(value.into())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalPolicyParseError(String);

impl fmt::Display for ApprovalPolicyParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid approval policy: {}", self.0)
    }
}
impl std::error::Error for ApprovalPolicyParseError {}

#[cfg(test)]
#[path = "approval_policy_tests.rs"]
mod tests;
