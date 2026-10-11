//! The durable identity of one coding job's budget policy.

use std::time::Duration;

use serde::{Deserialize, Serialize, de::Deserializer};
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::{
    Sha256Digest,
    control_wire::{self, ControlWireError},
};

/// The wire/schema version this crate writes and accepts for budget
/// policies. A record whose `version` field differs is refused as
/// [`BudgetPolicyError::UnsupportedVersion`].
pub const BUDGET_POLICY_VERSION: u32 = 1;

/// The six finite count ceilings that define a coding job's budget policy,
/// serialized with explicit declaration-order fields so the canonical JSON
/// bytes — and therefore the digest — are deterministic. No field may ever be
/// a map or `serde_json::Value`, which could reorder keys and change bytes.
///
/// Token, cost, and byte ceilings are absent from this count policy.
/// Optional verified token bounds are recorded through TokenBudgetPolicyV1;
/// this record never implies optional token enforcement. The deadline is stored as integer milliseconds; wall-clock
/// elapsed continuity across a restart is not claimed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BudgetLimitPolicyV1 {
    version: u32,
    /// Cumulative admitted logical receives.
    logical_requests: u64,
    /// Cumulative admitted physical sends.
    transport_attempts: u64,
    /// Cumulative admitted tool items.
    tool_calls: u64,
    /// Active model requests.
    model_slots: u64,
    /// Active child runs.
    child_slots: u64,
    /// Elapsed-time limit in milliseconds from owner construction.
    deadline_millis: u64,
}

impl BudgetLimitPolicyV1 {
    /// Validates and constructs a policy. Every limit must be finite
    /// positive: zero is refused for each count and for the deadline,
    /// mirroring the harness `CountLimits` finiteness check that
    /// `JobBudgetLedger::new` applies. No upper bound is invented here — the
    /// ledger defines none, and a policy-only cap would refuse limits the
    /// ledger admits.
    pub fn new(
        logical_requests: u64,
        transport_attempts: u64,
        tool_calls: u64,
        model_slots: u64,
        child_slots: u64,
        deadline_millis: u64,
    ) -> Result<Self, BudgetPolicyError> {
        if logical_requests == 0
            || transport_attempts == 0
            || tool_calls == 0
            || model_slots == 0
            || child_slots == 0
            || deadline_millis == 0
        {
            return Err(BudgetPolicyError::ZeroLimit);
        }
        Ok(Self {
            version: BUDGET_POLICY_VERSION,
            logical_requests,
            transport_attempts,
            tool_calls,
            model_slots,
            child_slots,
            deadline_millis,
        })
    }

    /// Typed entry point: bounds, gates the version, strict-decodes the wire
    /// record, and revalidates through [`BudgetLimitPolicyV1::new`] so the
    /// typed refusal reason (not a flattened wire error) is preserved.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, BudgetPolicyError> {
        control_wire::check_version(bytes, BUDGET_POLICY_VERSION).map_err(map_wire_error)?;
        let wire: WireBudgetLimitPolicy =
            serde_json::from_slice(bytes).map_err(|_| BudgetPolicyError::Malformed)?;
        Self::new(
            wire.logical_requests,
            wire.transport_attempts,
            wire.tool_calls,
            wire.model_slots,
            wire.child_slots,
            wire.deadline_millis,
        )
    }

    /// The exact bytes the digest is taken over: compact JSON, fields in
    /// declaration order, bounded like every control record.
    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, BudgetPolicyError> {
        control_wire::encode(self).map_err(map_wire_error)
    }

    /// SHA-256 over [`BudgetLimitPolicyV1::canonical_json_bytes`] — the
    /// policy's content identity for insert-once/compare-or-refuse storage.
    pub fn digest(&self) -> Sha256Digest {
        let bytes =
            serde_json::to_vec(self).expect("a fixed integer-only struct serializes infallibly");
        Sha256Digest::parse(&sha256_hex(&bytes)).expect("sha256 hex is always a valid digest")
    }

    /// The policy's wire version, always [`BUDGET_POLICY_VERSION`].
    pub fn version(&self) -> u32 {
        self.version
    }

    /// The cumulative count of admitted logical receives.
    pub fn logical_requests(&self) -> u64 {
        self.logical_requests
    }

    /// The cumulative count of admitted physical sends.
    pub fn transport_attempts(&self) -> u64 {
        self.transport_attempts
    }

    /// The cumulative count of admitted tool items.
    pub fn tool_calls(&self) -> u64 {
        self.tool_calls
    }

    /// The ceiling on concurrent model requests.
    pub fn model_slots(&self) -> u64 {
        self.model_slots
    }

    /// The ceiling on concurrent child runs.
    pub fn child_slots(&self) -> u64 {
        self.child_slots
    }

    /// The elapsed-time limit in integer milliseconds from owner
    /// construction.
    pub fn deadline_millis(&self) -> u64 {
        self.deadline_millis
    }

    /// The elapsed-time limit as a [`Duration`]; sub-millisecond precision
    /// is not claimed because the stored value is whole milliseconds.
    pub fn deadline(&self) -> Duration {
        Duration::from_millis(self.deadline_millis)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireBudgetLimitPolicy {
    version: u32,
    logical_requests: u64,
    transport_attempts: u64,
    tool_calls: u64,
    model_slots: u64,
    child_slots: u64,
    deadline_millis: u64,
}

impl<'de> Deserialize<'de> for BudgetLimitPolicyV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = WireBudgetLimitPolicy::deserialize(deserializer)?;
        if wire.version != BUDGET_POLICY_VERSION {
            return Err(serde::de::Error::custom(
                BudgetPolicyError::UnsupportedVersion(wire.version),
            ));
        }
        Self::new(
            wire.logical_requests,
            wire.transport_attempts,
            wire.tool_calls,
            wire.model_slots,
            wire.child_slots,
            wire.deadline_millis,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Why a budget policy record was rejected.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum BudgetPolicyError {
    #[error("unsupported coding budget policy version {0}")]
    UnsupportedVersion(u32),
    #[error("coding budget policy record is {0} bytes, over the 65536-byte limit")]
    Oversize(usize),
    #[error("coding budget policy record is malformed")]
    Malformed,
    #[error("budget policy limits must be finite positive; zero is refused")]
    ZeroLimit,
}

fn map_wire_error(error: ControlWireError) -> BudgetPolicyError {
    match error {
        ControlWireError::Oversize(bytes) => BudgetPolicyError::Oversize(bytes),
        ControlWireError::UnsupportedVersion(version) => {
            BudgetPolicyError::UnsupportedVersion(version)
        }
        ControlWireError::Malformed => BudgetPolicyError::Malformed,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(bytes);
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
#[path = "budget_policy_tests.rs"]
mod tests;
