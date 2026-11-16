use std::num::NonZeroU64;

use serde::{Deserialize, Serialize, de::Deserializer};

use super::budget::BudgetError;
use super::budget_values::{BudgetObservation, validate_entries};
use super::{JobId, control_wire};

pub const BUDGET_SETTLEMENT_VERSION: u32 = 1;

/// Whether an effect's accounting completed or still needs reconciliation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BudgetEffectState {
    Settled,
    Uncertain,
}

/// Observed usage for one earlier reservation. The store checks that the
/// observation keys match their reservation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BudgetSettlementV1 {
    version: u32,
    job_id: JobId,
    reservation_sequence: NonZeroU64,
    settlement_sequence: NonZeroU64,
    effect_state: BudgetEffectState,
    observations: Vec<BudgetObservation>,
}

impl BudgetSettlementV1 {
    pub fn new(
        job_id: JobId,
        reservation_sequence: NonZeroU64,
        settlement_sequence: NonZeroU64,
        effect_state: BudgetEffectState,
        observations: Vec<BudgetObservation>,
    ) -> Result<Self, BudgetError> {
        validate_entries(
            &observations,
            |observation| (observation.dimension(), observation.endpoint()),
            BudgetError::EmptyObservations,
            BudgetError::TooManyObservations,
            BudgetError::DuplicateObservationKey,
        )?;
        let settlement = Self {
            version: BUDGET_SETTLEMENT_VERSION,
            job_id,
            reservation_sequence,
            settlement_sequence,
            effect_state,
            observations,
        };
        control_wire::encode(&settlement).map_err(super::budget::map_wire_error)?;
        Ok(settlement)
    }

    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, BudgetError> {
        control_wire::decode_versioned(bytes, BUDGET_SETTLEMENT_VERSION)
            .map_err(super::budget::map_wire_error)
    }

    pub fn to_json_bytes(&self) -> Result<Vec<u8>, BudgetError> {
        control_wire::encode(self).map_err(super::budget::map_wire_error)
    }

    pub fn job_id(&self) -> &JobId {
        &self.job_id
    }

    pub fn reservation_sequence(&self) -> NonZeroU64 {
        self.reservation_sequence
    }

    pub fn settlement_sequence(&self) -> NonZeroU64 {
        self.settlement_sequence
    }

    pub fn effect_state(&self) -> BudgetEffectState {
        self.effect_state
    }

    pub fn observations(&self) -> &[BudgetObservation] {
        &self.observations
    }

    fn from_wire(wire: WireBudgetSettlement) -> Result<Self, BudgetError> {
        if wire.version != BUDGET_SETTLEMENT_VERSION {
            return Err(BudgetError::UnsupportedVersion(wire.version));
        }
        Self::new(
            wire.job_id,
            wire.reservation_sequence,
            wire.settlement_sequence,
            wire.effect_state,
            wire.observations,
        )
    }
}

impl<'de> Deserialize<'de> for BudgetSettlementV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::from_wire(WireBudgetSettlement::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireBudgetSettlement {
    version: u32,
    job_id: JobId,
    reservation_sequence: NonZeroU64,
    settlement_sequence: NonZeroU64,
    effect_state: BudgetEffectState,
    observations: Vec<BudgetObservation>,
}
