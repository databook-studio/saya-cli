use std::num::NonZeroU64;

use serde::{Deserialize, Serialize, de::Deserializer};
use thiserror::Error;

use super::budget_values::{BudgetCharge, validate_entries};
use super::control_wire::{self, ControlWireError};
use super::{AgentId, InvocationId, JobId};

pub const BUDGET_RESERVATION_VERSION: u32 = 1;

/// Root-owned upper-bound amounts persisted before an effect is admitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BudgetReservationV1 {
    version: u32,
    job_id: JobId,
    reservation_sequence: NonZeroU64,
    agent_id: Option<AgentId>,
    invocation_id: Option<InvocationId>,
    charges: Vec<BudgetCharge>,
}

impl BudgetReservationV1 {
    pub fn new(
        job_id: JobId,
        reservation_sequence: NonZeroU64,
        agent_id: Option<AgentId>,
        invocation_id: Option<InvocationId>,
        charges: Vec<BudgetCharge>,
    ) -> Result<Self, BudgetError> {
        validate_child_identity(agent_id.as_ref(), invocation_id.as_ref())?;
        validate_entries(
            &charges,
            |charge| (charge.dimension(), charge.endpoint()),
            BudgetError::EmptyCharges,
            BudgetError::TooManyCharges,
            BudgetError::DuplicateChargeKey,
        )?;
        let reservation = Self {
            version: BUDGET_RESERVATION_VERSION,
            job_id,
            reservation_sequence,
            agent_id,
            invocation_id,
            charges,
        };
        control_wire::encode(&reservation).map_err(map_wire_error)?;
        Ok(reservation)
    }

    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, BudgetError> {
        control_wire::decode_versioned(bytes, BUDGET_RESERVATION_VERSION).map_err(map_wire_error)
    }

    pub fn to_json_bytes(&self) -> Result<Vec<u8>, BudgetError> {
        control_wire::encode(self).map_err(map_wire_error)
    }

    pub fn job_id(&self) -> &JobId {
        &self.job_id
    }

    pub fn reservation_sequence(&self) -> NonZeroU64 {
        self.reservation_sequence
    }

    pub fn agent_id(&self) -> Option<&AgentId> {
        self.agent_id.as_ref()
    }

    pub fn invocation_id(&self) -> Option<&InvocationId> {
        self.invocation_id.as_ref()
    }

    pub fn charges(&self) -> &[BudgetCharge] {
        &self.charges
    }

    fn from_wire(wire: WireBudgetReservation) -> Result<Self, BudgetError> {
        if wire.version != BUDGET_RESERVATION_VERSION {
            return Err(BudgetError::UnsupportedVersion(wire.version));
        }
        Self::new(
            wire.job_id,
            wire.reservation_sequence,
            wire.agent_id,
            wire.invocation_id,
            wire.charges,
        )
    }
}

impl<'de> Deserialize<'de> for BudgetReservationV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::from_wire(WireBudgetReservation::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
}

fn validate_child_identity(
    agent_id: Option<&AgentId>,
    invocation_id: Option<&InvocationId>,
) -> Result<(), BudgetError> {
    if invocation_id.is_some() && agent_id.is_none() {
        return Err(BudgetError::InvalidIdentity);
    }
    Ok(())
}

pub(super) fn map_wire_error(error: ControlWireError) -> BudgetError {
    match error {
        ControlWireError::Oversize(bytes) => BudgetError::Oversize(bytes),
        ControlWireError::UnsupportedVersion(version) => BudgetError::UnsupportedVersion(version),
        ControlWireError::Malformed => BudgetError::Malformed,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireBudgetReservation {
    version: u32,
    job_id: JobId,
    reservation_sequence: NonZeroU64,
    agent_id: Option<AgentId>,
    invocation_id: Option<InvocationId>,
    charges: Vec<BudgetCharge>,
}

/// Why a budget record was rejected.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum BudgetError {
    #[error("budget record is {0} bytes, over the 65536-byte limit")]
    Oversize(usize),
    #[error("unsupported coding budget version {0}")]
    UnsupportedVersion(u32),
    #[error("coding budget record is malformed")]
    Malformed,
    #[error("an invocation identity requires an agent identity")]
    InvalidIdentity,
    #[error("budget reservation must contain at least one charge")]
    EmptyCharges,
    #[error("budget reservation exceeds the 64-charge limit")]
    TooManyCharges,
    #[error("budget charge keys must be unique")]
    DuplicateChargeKey,
    #[error("budget settlement must contain at least one observation")]
    EmptyObservations,
    #[error("budget settlement exceeds the 64-observation limit")]
    TooManyObservations,
    #[error("budget observation keys must be unique")]
    DuplicateObservationKey,
    #[error("endpoint reference must be 1-128 safe ASCII bytes")]
    InvalidEndpoint,
    #[error("endpoint reference does not match the budget dimension")]
    EndpointScopeMismatch,
    #[error("price metadata digest does not match the cost dimension")]
    InvalidPriceMetadata,
}
