use std::{fmt, str::FromStr};

/// Controls whether the agent may execute bounded read-only queries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ApprovalPolicy {
    #[default]
    Ask,
    ReadOnly,
    Never,
}

impl FromStr for ApprovalPolicy {
    type Err = ApprovalPolicyParseError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "ask" => Ok(Self::Ask),
            "read-only" => Ok(Self::ReadOnly),
            "never" => Ok(Self::Never),
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
