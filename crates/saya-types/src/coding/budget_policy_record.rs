//! Version-dispatched durable budget policy records. V1 remains byte-for-byte intact.

use serde::{Deserialize, Serialize};

use super::{
    BudgetLimitPolicyV1, BudgetLimitPolicyV2, BudgetPolicyError, CodingBudgetMode, Sha256Digest,
    control_wire::{self, ControlWireError},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodingBudgetPolicy {
    BoundedV1(BudgetLimitPolicyV1),
    UncappedV2(BudgetLimitPolicyV2),
}

impl CodingBudgetPolicy {
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, BudgetPolicyError> {
        if bytes.len() > control_wire::MAX_CONTROL_RECORD_BYTES {
            return Err(BudgetPolicyError::Oversize(bytes.len()));
        }
        let probe: VersionProbe =
            serde_json::from_slice(bytes).map_err(|_| BudgetPolicyError::Malformed)?;
        match probe.version {
            Some(1) => BudgetLimitPolicyV1::from_json_bytes(bytes).map(Self::BoundedV1),
            Some(2) => BudgetLimitPolicyV2::from_json_bytes(bytes).map(Self::UncappedV2),
            Some(version) => Err(BudgetPolicyError::UnsupportedVersion(version)),
            None => Err(BudgetPolicyError::Malformed),
        }
    }

    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, BudgetPolicyError> {
        match self {
            Self::BoundedV1(policy) => policy.canonical_json_bytes(),
            Self::UncappedV2(policy) => policy.canonical_json_bytes(),
        }
    }

    pub fn digest(&self) -> Sha256Digest {
        match self {
            Self::BoundedV1(policy) => policy.digest(),
            Self::UncappedV2(policy) => policy.digest(),
        }
    }

    pub fn version(&self) -> u32 {
        match self {
            Self::BoundedV1(policy) => policy.version(),
            Self::UncappedV2(policy) => policy.version(),
        }
    }

    pub fn budget_mode(&self) -> CodingBudgetMode {
        match self {
            Self::BoundedV1(_) => CodingBudgetMode::Bounded,
            Self::UncappedV2(policy) => policy.budget_mode(),
        }
    }

    pub fn bounded_v1(&self) -> Option<&BudgetLimitPolicyV1> {
        match self {
            Self::BoundedV1(policy) => Some(policy),
            Self::UncappedV2(_) => None,
        }
    }

    pub fn uncapped_v2(&self) -> Option<&BudgetLimitPolicyV2> {
        match self {
            Self::BoundedV1(_) => None,
            Self::UncappedV2(policy) => Some(policy),
        }
    }
}

impl Serialize for CodingBudgetPolicy {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::BoundedV1(policy) => policy.serialize(serializer),
            Self::UncappedV2(policy) => policy.serialize(serializer),
        }
    }
}

impl PartialEq<BudgetLimitPolicyV1> for CodingBudgetPolicy {
    fn eq(&self, other: &BudgetLimitPolicyV1) -> bool {
        matches!(self, Self::BoundedV1(policy) if policy == other)
    }
}

impl PartialEq<CodingBudgetPolicy> for BudgetLimitPolicyV1 {
    fn eq(&self, other: &CodingBudgetPolicy) -> bool {
        other == self
    }
}

#[derive(Deserialize)]
struct VersionProbe {
    version: Option<u32>,
}

pub(super) fn map_wire_error(error: ControlWireError) -> BudgetPolicyError {
    match error {
        ControlWireError::Oversize(bytes) => BudgetPolicyError::Oversize(bytes),
        ControlWireError::UnsupportedVersion(version) => {
            BudgetPolicyError::UnsupportedVersion(version)
        }
        ControlWireError::Malformed => BudgetPolicyError::Malformed,
    }
}
