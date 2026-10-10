use std::{collections::HashSet, num::NonZeroU64};

use serde::{Deserialize, Serialize, de::Deserializer};

use super::{BudgetError, Sha256Digest};

pub const MAX_BUDGET_ENTRIES: usize = 64;
pub const MAX_BUDGET_ENDPOINT_BYTES: usize = 128;

/// Resource charged by one root-owned coding reservation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BudgetDimension {
    LogicalRequests,
    TransportAttempts,
    ToolCalls,
    InputTokens,
    OutputTokens,
    ElapsedMilliseconds,
    OutputBytes,
    ChildSlots,
    ModelRequestSlots,
    CommandSlots,
    RepositoryMutations,
    IntegrationWriters,
    CostMicrousd,
}

/// How much usage is known after an effect. Absence and unavailability are
/// explicit; a reported zero remains a measured zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "status",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
#[non_exhaustive]
pub enum BudgetObservationValue {
    Measured(u64),
    Estimated(u64),
    Unknown,
    Unavailable,
}

/// One positive upper-bound charge reserved before an effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BudgetCharge {
    dimension: BudgetDimension,
    endpoint: Option<String>,
    price_metadata_sha256: Option<Sha256Digest>,
    reserved: NonZeroU64,
}

impl BudgetCharge {
    pub fn new(
        dimension: BudgetDimension,
        endpoint: Option<String>,
        price_metadata_sha256: Option<Sha256Digest>,
        reserved: NonZeroU64,
    ) -> Result<Self, BudgetError> {
        validate_scope(
            dimension,
            endpoint.as_deref(),
            price_metadata_sha256.as_ref(),
        )?;
        Ok(Self {
            dimension,
            endpoint,
            price_metadata_sha256,
            reserved,
        })
    }

    pub fn dimension(&self) -> BudgetDimension {
        self.dimension
    }

    pub fn endpoint(&self) -> Option<&str> {
        self.endpoint.as_deref()
    }

    pub fn price_metadata_sha256(&self) -> Option<&Sha256Digest> {
        self.price_metadata_sha256.as_ref()
    }

    pub fn reserved(&self) -> NonZeroU64 {
        self.reserved
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireBudgetCharge {
    dimension: BudgetDimension,
    endpoint: Option<String>,
    price_metadata_sha256: Option<Sha256Digest>,
    reserved: NonZeroU64,
}

impl<'de> Deserialize<'de> for BudgetCharge {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = WireBudgetCharge::deserialize(deserializer)?;
        Self::new(
            wire.dimension,
            wire.endpoint,
            wire.price_metadata_sha256,
            wire.reserved,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// One observed amount for a previously reserved dimension and scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BudgetObservation {
    dimension: BudgetDimension,
    endpoint: Option<String>,
    price_metadata_sha256: Option<Sha256Digest>,
    value: BudgetObservationValue,
}

impl BudgetObservation {
    pub fn new(
        dimension: BudgetDimension,
        endpoint: Option<String>,
        price_metadata_sha256: Option<Sha256Digest>,
        value: BudgetObservationValue,
    ) -> Result<Self, BudgetError> {
        validate_scope(
            dimension,
            endpoint.as_deref(),
            price_metadata_sha256.as_ref(),
        )?;
        Ok(Self {
            dimension,
            endpoint,
            price_metadata_sha256,
            value,
        })
    }

    pub fn dimension(&self) -> BudgetDimension {
        self.dimension
    }

    pub fn endpoint(&self) -> Option<&str> {
        self.endpoint.as_deref()
    }

    pub fn price_metadata_sha256(&self) -> Option<&Sha256Digest> {
        self.price_metadata_sha256.as_ref()
    }

    pub fn value(&self) -> BudgetObservationValue {
        self.value
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireBudgetObservation {
    dimension: BudgetDimension,
    endpoint: Option<String>,
    price_metadata_sha256: Option<Sha256Digest>,
    value: BudgetObservationValue,
}

impl<'de> Deserialize<'de> for BudgetObservation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = WireBudgetObservation::deserialize(deserializer)?;
        Self::new(
            wire.dimension,
            wire.endpoint,
            wire.price_metadata_sha256,
            wire.value,
        )
        .map_err(serde::de::Error::custom)
    }
}

pub(super) fn validate_entries<T>(
    entries: &[T],
    key: impl Fn(&T) -> (BudgetDimension, Option<&str>),
    empty: BudgetError,
    too_many: BudgetError,
    duplicate: BudgetError,
) -> Result<(), BudgetError> {
    if entries.is_empty() {
        return Err(empty);
    }
    if entries.len() > MAX_BUDGET_ENTRIES {
        return Err(too_many);
    }
    let mut seen = HashSet::with_capacity(entries.len());
    if entries.iter().any(|entry| !seen.insert(key(entry))) {
        return Err(duplicate);
    }
    Ok(())
}

fn validate_scope(
    dimension: BudgetDimension,
    endpoint: Option<&str>,
    price_metadata_sha256: Option<&Sha256Digest>,
) -> Result<(), BudgetError> {
    let endpoint_required = matches!(
        dimension,
        BudgetDimension::InputTokens
            | BudgetDimension::OutputTokens
            | BudgetDimension::CostMicrousd
    );
    if endpoint.is_some() != endpoint_required {
        return Err(BudgetError::EndpointScopeMismatch);
    }
    if let Some(endpoint) = endpoint
        && (endpoint.is_empty()
            || endpoint.len() > MAX_BUDGET_ENDPOINT_BYTES
            || !endpoint
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_')))
    {
        return Err(BudgetError::InvalidEndpoint);
    }
    let price_required = dimension == BudgetDimension::CostMicrousd;
    if price_metadata_sha256.is_some() != price_required {
        return Err(BudgetError::InvalidPriceMetadata);
    }
    Ok(())
}
