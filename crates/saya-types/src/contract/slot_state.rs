//! The persisted lifecycle states a knowledge slot's value can be in.

use serde::{Deserialize, Serialize};

/// The three persisted states a knowledge slot's value can be in. Distinct
/// from `ClaimStatus`, which is still in use and whose `Contradicted` variant
/// is load-bearing in `saya-store`'s transition rules; this adds the new
/// vocabulary beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum KnowledgeState {
    Pending,
    Active,
    Dismissed,
}

impl KnowledgeState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Dismissed => "dismissed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "active" => Some(Self::Active),
            "dismissed" => Some(Self::Dismissed),
            _ => None,
        }
    }
}
