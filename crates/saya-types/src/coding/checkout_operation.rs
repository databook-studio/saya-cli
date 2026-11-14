use std::num::NonZeroU64;

use serde::{Deserialize, Serialize, de::Deserializer};

use super::checkouts::{CheckoutDtoError, map_wire_error};
use super::control_wire;
use super::{JobId, ManagedCheckoutId, RepositoryMutationAction};

pub const CHECKOUT_OPERATION_VERSION: u32 = 1;

/// One durable checkout effect, keyed by the original intent event sequence.
/// Later outcome events receive their own event sequence outside this record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckoutOperationV1 {
    version: u32,
    job_id: JobId,
    operation_sequence: NonZeroU64,
    managed_checkout_id: ManagedCheckoutId,
    action: RepositoryMutationAction,
    state: CheckoutOperationState,
}

impl CheckoutOperationV1 {
    pub fn new(
        job_id: JobId,
        operation_sequence: NonZeroU64,
        managed_checkout_id: ManagedCheckoutId,
        action: RepositoryMutationAction,
        state: CheckoutOperationState,
    ) -> Result<Self, CheckoutDtoError> {
        let operation = Self {
            version: CHECKOUT_OPERATION_VERSION,
            job_id,
            operation_sequence,
            managed_checkout_id,
            action,
            state,
        };
        control_wire::encode(&operation).map_err(map_wire_error)?;
        Ok(operation)
    }

    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, CheckoutDtoError> {
        control_wire::decode_versioned(bytes, CHECKOUT_OPERATION_VERSION).map_err(map_wire_error)
    }

    pub fn to_json_bytes(&self) -> Result<Vec<u8>, CheckoutDtoError> {
        control_wire::encode(self).map_err(map_wire_error)
    }

    pub fn job_id(&self) -> &JobId {
        &self.job_id
    }

    pub fn operation_sequence(&self) -> NonZeroU64 {
        self.operation_sequence
    }

    pub fn managed_checkout_id(&self) -> &ManagedCheckoutId {
        &self.managed_checkout_id
    }

    pub fn action(&self) -> RepositoryMutationAction {
        self.action
    }

    pub fn state(&self) -> CheckoutOperationState {
        self.state
    }

    fn from_wire(wire: WireCheckoutOperation) -> Result<Self, CheckoutDtoError> {
        if wire.version != CHECKOUT_OPERATION_VERSION {
            return Err(CheckoutDtoError::UnsupportedVersion(wire.version));
        }
        Self::new(
            wire.job_id,
            wire.operation_sequence,
            wire.managed_checkout_id,
            wire.action,
            wire.state,
        )
    }
}

impl<'de> Deserialize<'de> for CheckoutOperationV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::from_wire(WireCheckoutOperation::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireCheckoutOperation {
    version: u32,
    job_id: JobId,
    operation_sequence: NonZeroU64,
    managed_checkout_id: ManagedCheckoutId,
    action: RepositoryMutationAction,
    state: CheckoutOperationState,
}

/// The durable knowledge about whether a checkout effect ran or is uncertain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CheckoutOperationState {
    Intent,
    Completed,
    Failed,
    Uncertain,
}
