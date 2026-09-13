//! The approval modes' vocabulary: one spelling per mode, parsed here and
//! carried everywhere else as the type.

use std::{fmt, str::FromStr};

/// Controls what the approval engine answers for a tool call. `ask` renders
/// a prompt per call; `read-only` auto-approves read-shaped tools only and
/// denies the rest; `never` denies everything; `bypass` allows every call
/// without asking — the per-call consent given once at launch, with every
/// structural guard (the SQL safety layer, the sandbox, the allowlists)
/// untouched.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ApprovalPolicy {
    #[default]
    Ask,
    ReadOnly,
    Never,
    Bypass,
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
