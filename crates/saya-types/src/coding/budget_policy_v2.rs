//! Versioned policy identity for jobs that use no configured aggregate budget ceilings.

use serde::{Deserialize, Serialize, de::Deserializer};

use super::{
    BudgetPolicyError, CodingBudgetMode, Sha256Digest, budget_policy::sha256_hex,
    budget_policy_record::map_wire_error, control_wire,
};

pub const BUDGET_POLICY_V2_VERSION: u32 = 2;

/// A durable uncapped policy. Aggregate ceilings are explicit nulls; finite
/// model and child concurrency remain part of the job identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BudgetLimitPolicyV2 {
    version: u32,
    budget_mode: CodingBudgetMode,
    logical_requests: Option<u64>,
    transport_attempts: Option<u64>,
    tool_calls: Option<u64>,
    per_loop_tokens: Option<u64>,
    job_tokens: Option<u64>,
    elapsed_millis: Option<u64>,
    model_slots: u64,
    child_slots: u64,
}

impl BudgetLimitPolicyV2 {
    pub fn uncapped(model_slots: u64, child_slots: u64) -> Result<Self, BudgetPolicyError> {
        if model_slots == 0 || child_slots == 0 {
            return Err(BudgetPolicyError::ZeroLimit);
        }
        Ok(Self {
            version: BUDGET_POLICY_V2_VERSION,
            budget_mode: CodingBudgetMode::Uncapped,
            logical_requests: None,
            transport_attempts: None,
            tool_calls: None,
            per_loop_tokens: None,
            job_tokens: None,
            elapsed_millis: None,
            model_slots,
            child_slots,
        })
    }

    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, BudgetPolicyError> {
        control_wire::check_version(bytes, BUDGET_POLICY_V2_VERSION).map_err(map_wire_error)?;
        let wire: WireBudgetLimitPolicyV2 =
            serde_json::from_slice(bytes).map_err(|_| BudgetPolicyError::Malformed)?;
        if wire.version != BUDGET_POLICY_V2_VERSION {
            return Err(BudgetPolicyError::UnsupportedVersion(wire.version));
        }
        if wire.budget_mode != CodingBudgetMode::Uncapped
            || wire.logical_requests.is_some()
            || wire.transport_attempts.is_some()
            || wire.tool_calls.is_some()
            || wire.per_loop_tokens.is_some()
            || wire.job_tokens.is_some()
            || wire.elapsed_millis.is_some()
        {
            return Err(BudgetPolicyError::InvalidUncappedPolicy);
        }
        let policy = Self::uncapped(wire.model_slots, wire.child_slots)?;
        if policy.canonical_json_bytes()?.as_slice() != bytes {
            return Err(BudgetPolicyError::Malformed);
        }
        Ok(policy)
    }

    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, BudgetPolicyError> {
        control_wire::encode(self).map_err(map_wire_error)
    }

    pub fn digest(&self) -> Sha256Digest {
        let bytes = serde_json::to_vec(self).expect("fixed policy fields serialize infallibly");
        Sha256Digest::parse(&sha256_hex(&bytes)).expect("sha256 hex is a valid digest")
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn budget_mode(&self) -> CodingBudgetMode {
        self.budget_mode
    }

    pub fn logical_requests(&self) -> Option<u64> {
        self.logical_requests
    }

    pub fn transport_attempts(&self) -> Option<u64> {
        self.transport_attempts
    }

    pub fn tool_calls(&self) -> Option<u64> {
        self.tool_calls
    }

    pub fn per_loop_tokens(&self) -> Option<u64> {
        self.per_loop_tokens
    }

    pub fn job_tokens(&self) -> Option<u64> {
        self.job_tokens
    }

    pub fn elapsed_millis(&self) -> Option<u64> {
        self.elapsed_millis
    }

    pub fn model_slots(&self) -> u64 {
        self.model_slots
    }

    pub fn child_slots(&self) -> u64 {
        self.child_slots
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireBudgetLimitPolicyV2 {
    version: u32,
    budget_mode: CodingBudgetMode,
    #[serde(deserialize_with = "required_option")]
    logical_requests: Option<u64>,
    #[serde(deserialize_with = "required_option")]
    transport_attempts: Option<u64>,
    #[serde(deserialize_with = "required_option")]
    tool_calls: Option<u64>,
    #[serde(deserialize_with = "required_option")]
    per_loop_tokens: Option<u64>,
    #[serde(deserialize_with = "required_option")]
    job_tokens: Option<u64>,
    #[serde(deserialize_with = "required_option")]
    elapsed_millis: Option<u64>,
    model_slots: u64,
    child_slots: u64,
}

fn required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

#[cfg(test)]
#[path = "budget_policy_v2_tests.rs"]
mod tests;
