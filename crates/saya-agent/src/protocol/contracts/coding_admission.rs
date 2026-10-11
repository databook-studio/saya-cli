use std::{error::Error, fmt, num::NonZeroU64};

use async_trait::async_trait;

use crate::TokenUsage;

use super::ToolEffect;
use super::coding_usage::PhysicalAttempt;

/// Owner-assigned identity for one held effect reservation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EffectPermit(NonZeroU64);

impl EffectPermit {
    pub const fn new(id: NonZeroU64) -> Self {
        Self(id)
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// What is known about an admitted logical effect when its reservation settles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EffectOutcome {
    /// The loop confirmed cancellation before invoking the provider or executor.
    NotStarted,
    /// The provider returned a logical response or the executor returned success.
    Succeeded,
    /// Dispatch may have occurred, but completion did not prove the effect outcome.
    Unknown,
}

/// Source-owned outcome data attached to one effect permit.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EffectSettlement {
    ModelRequest {
        outcome: EffectOutcome,
        reported_usage: Option<TokenUsage>,
        usage_complete: bool,
    },
    Tool {
        outcome: EffectOutcome,
    },
    /// One transport send under a logical model request. A failed response
    /// without usage remains Unknown rather than known zero.
    PhysicalAttempt {
        attempt: PhysicalAttempt,
        outcome: EffectOutcome,
        reported_usage: Option<TokenUsage>,
        usage_complete: bool,
    },
}

/// A cause-preserving owner error whose display and debug forms omit its text.
pub struct EffectOwnerFailure(Box<dyn Error + Send + Sync + 'static>);

impl EffectOwnerFailure {
    pub fn new(source: impl Error + Send + Sync + 'static) -> Self {
        Self(Box::new(source))
    }

    pub(crate) fn contract_violation() -> Self {
        Self::new(EffectAdmissionContractViolation)
    }
}

impl fmt::Debug for EffectOwnerFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EffectOwnerFailure { source: [redacted] }")
    }
}

impl fmt::Display for EffectOwnerFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("effect admission owner failed")
    }
}

impl Error for EffectOwnerFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.0.as_ref())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("effect admission owner violated its permit contract")]
struct EffectAdmissionContractViolation;

/// Why an owner declined to issue a reservation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EffectAdmissionError {
    #[error("effect admission denied")]
    Denied,
    #[error(transparent)]
    OwnerFailure(#[from] EffectOwnerFailure),
    #[error("provider cannot report physical transport attempts")]
    UnsupportedPhysicalCoverage,
}

/// A borrowed root-owned gate for logical requests, optional physical sends,
/// and executable tools. `with_effect_admission` stays logical-only;
/// `with_physical_effect_admission` fails closed unless the owner implements
/// physical admission.
///
/// Tool-batch admission is atomic: on success the returned permits must have
/// exactly the input length and order; the agent supplies at most
/// [`crate::MAX_TOOL_CALLS_PER_RESPONSE`] effects. Admission and settlement
/// operations should be bounded and recoverable if their caller task is
/// dropped. The loop awaits them without racing child cancellation. A permit
/// dropped without settlement remains held/unknown at its owner; dropping it
/// never proves no effect. One model permit covers one logical receive; it does
/// not count physical retries or hidden auxiliary provider calls unless the
/// strict physical builder is used. On model
/// settlements, `usage_complete` describes whether all attempts visible to the
/// agent receive loop reported scalar token totals; observed partial totals
/// stay present. It is not proof that provider-internal retries were observed.
#[async_trait]
pub trait EffectAdmission: Send + Sync {
    async fn admit_model_request(&self) -> Result<EffectPermit, EffectAdmissionError>;

    async fn admit_tool_batch(
        &self,
        effects: &[ToolEffect],
    ) -> Result<Vec<EffectPermit>, EffectAdmissionError>;

    /// Reserves one actual provider transport send under its logical request.
    /// Existing owners remain logical-only unless they explicitly implement
    /// this operation; strict provider coverage fails closed by default.
    async fn admit_physical_attempt(
        &self,
        _attempt: PhysicalAttempt,
    ) -> Result<EffectPermit, EffectAdmissionError> {
        Err(EffectAdmissionError::UnsupportedPhysicalCoverage)
    }

    /// Native strict owners validate producer-supplied actual wire settings.
    /// Missing metadata remains compatible only for non-token-constrained owners.
    async fn admit_physical_request(
        &self,
        attempt: PhysicalAttempt,
        _request: Option<&crate::PhysicalRequestMetadata>,
    ) -> Result<EffectPermit, EffectAdmissionError> {
        self.admit_physical_attempt(attempt).await
    }

    /// Source dispatch/finish observation for an admitted physical attempt.
    /// Default owners collect no spans; durable consumers must report missing
    /// observations as incomplete. This never grants or settles an effect.
    async fn observe_physical_attempt(
        &self,
        _permit: EffectPermit,
        _attempt: PhysicalAttempt,
        _observation: crate::PhysicalAttemptObservation,
    ) -> Result<(), EffectOwnerFailure> {
        Ok(())
    }

    async fn settle(
        &self,
        permit: EffectPermit,
        settlement: EffectSettlement,
    ) -> Result<(), EffectOwnerFailure>;
}
