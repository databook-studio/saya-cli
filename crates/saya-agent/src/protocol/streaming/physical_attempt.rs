use super::CancellationToken;
use crate::{
    CancellationReason, EffectAdmission, EffectAdmissionError, EffectCallKind, EffectOutcome,
    EffectOwnerFailure, EffectPermit, EffectSettlement, PhysicalAttempt, ProviderError, TokenUsage,
};
use std::error::Error;

const MAX_PHYSICAL_ATTEMPTS_PER_RECEIVE: u8 = 16;

/// Failure from a controlled provider call, kept distinct from recoverable
/// transport errors so admission failures cannot enter provider retry policy.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ControlledProviderError {
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error("provider does not expose physical transport attempts")]
    UnsupportedCoverage,
    #[error("physical transport attempt denied")]
    AdmissionDenied,
    #[error(transparent)]
    OwnerFailure(#[from] EffectOwnerFailure),
    #[error("provider exceeded the physical attempt bound")]
    AttemptLimit,
}

/// Per-logical-request admission state passed to controlled provider calls.
/// Providers admit each send before dispatch and settle completed responses
/// after their stream has been consumed by the agent loop.
pub struct PhysicalAttemptControl<'a> {
    owner: &'a dyn EffectAdmission,
    request_metadata: Option<crate::PhysicalRequestMetadata>,
    cancellation: &'a CancellationToken,
    parent: EffectPermit,
    kind: EffectCallKind,
    request_correlation: Option<crate::PhysicalRequestCorrelation>,
    next_ordinal: u8,
    active: Option<(PhysicalAttempt, EffectPermit, bool)>,
    pending_response: Option<(PhysicalAttempt, EffectPermit)>,
    dispatched_any: bool,
    dispatch_started_ns: Option<u64>,
    usage_complete: bool,
}

impl<'a> PhysicalAttemptControl<'a> {
    pub(crate) fn new(
        owner: &'a dyn EffectAdmission,
        cancellation: &'a CancellationToken,
        parent: EffectPermit,
        kind: EffectCallKind,
        request_correlation: Option<crate::PhysicalRequestCorrelation>,
    ) -> Self {
        Self {
            owner,
            request_metadata: None,
            cancellation,
            parent,
            kind,
            request_correlation,
            next_ordinal: 0,
            active: None,
            pending_response: None,
            dispatched_any: false,
            dispatch_started_ns: None,
            usage_complete: true,
        }
    }

    /// Builtin providers bind the actual final endpoint/model/output projection.
    pub fn bind_request(
        &mut self,
        request: crate::PhysicalRequestMetadata,
    ) -> Result<(), ControlledProviderError> {
        if self.active.is_some() || self.pending_response.is_some() {
            return Err(self.contract_violation());
        }
        let mut request = request;
        request.set_correlation(self.request_correlation.clone());
        self.request_metadata = Some(request);
        Ok(())
    }

    /// Adds the OpenAI-compatible adapter's existing cache-key digest to the
    /// local observation metadata. It does not alter the wire key or body.
    pub fn bind_cache_key_digest(
        &mut self,
        digest: Option<u64>,
    ) -> Result<(), ControlledProviderError> {
        if self.active.is_some() || self.pending_response.is_some() {
            return Err(self.contract_violation());
        }
        if let Some(correlation) = self.request_correlation.as_mut() {
            correlation.set_cache_key_digest(digest);
        }
        self.refresh_request_correlation();
        Ok(())
    }

    /// Records the serialized provider projection size without retaining its
    /// content. Adapters with a precise wire projection may replace the neutral
    /// preflight byte count before dispatch.
    pub fn bind_projection_size_bytes(
        &mut self,
        size: usize,
    ) -> Result<(), ControlledProviderError> {
        if self.active.is_some() || self.pending_response.is_some() {
            return Err(self.contract_violation());
        }
        if let Some(correlation) = self.request_correlation.as_mut() {
            correlation.set_projection_size_bytes(size);
        }
        self.refresh_request_correlation();
        Ok(())
    }

    /// Reserves the next physical send. The caller must settle it or retain it
    /// as unknown before another send can be admitted.
    pub async fn admit_before_send(&mut self) -> Result<(), ControlledProviderError> {
        if self.active.is_some() || self.pending_response.is_some() {
            return Err(ControlledProviderError::OwnerFailure(
                EffectOwnerFailure::new(PhysicalAttemptContractViolation),
            ));
        }
        if self.cancellation.is_cancelled() {
            return Err(ProviderError::Cancelled.into());
        }
        if self.next_ordinal >= MAX_PHYSICAL_ATTEMPTS_PER_RECEIVE {
            return Err(ControlledProviderError::AttemptLimit);
        }
        let attempt = PhysicalAttempt::new(self.parent, self.kind, self.next_ordinal);
        self.next_ordinal += 1;
        let permit = match self
            .owner
            .admit_physical_request(attempt, self.request_metadata.as_ref())
            .await
        {
            Ok(permit) => permit,
            Err(EffectAdmissionError::Denied) => {
                return Err(ControlledProviderError::AdmissionDenied);
            }
            Err(EffectAdmissionError::OwnerFailure(error)) => {
                self.cancellation
                    .cancel_with_reason(CancellationReason::OwnerFailure);
                return Err(ControlledProviderError::OwnerFailure(error));
            }
            Err(EffectAdmissionError::UnsupportedPhysicalCoverage) => {
                return Err(ControlledProviderError::UnsupportedCoverage);
            }
        };
        self.active = Some((attempt, permit, false));
        if self.cancellation.is_cancelled() {
            // Leave the just-admitted attempt live: the loop settles an
            // establishment it dropped on cancellation, and settling here
            // would race that drop midway through the durable append.
            return Err(ProviderError::Cancelled.into());
        }
        Ok(())
    }

    /// Marks the send as potentially dispatched before polling its future.
    pub fn dispatch_started(&mut self) -> Result<(), ControlledProviderError> {
        let Some((_, _, dispatched)) = self.active.as_mut() else {
            return Err(self.contract_violation());
        };
        if *dispatched {
            return Err(self.contract_violation());
        }
        self.dispatch_started_ns = Some(super::physical_observation::stamp()?);
        *dispatched = true;
        self.dispatched_any = true;
        Ok(())
    }

    /// Settles whatever a send future that was dropped mid-establishment left
    /// behind. A held response is unknown; a still-active attempt is settled
    /// by its dispatch state; an attempt that never began needs no settlement.
    pub(crate) async fn settle_aborted_establishment(
        &mut self,
    ) -> Result<(), ControlledProviderError> {
        if self.pending_response.is_some() {
            return self
                .settle_pending_response(EffectOutcome::Unknown, None)
                .await;
        }
        if self.active.is_some() {
            return self.settle_cancelled().await;
        }
        Ok(())
    }

    /// The biased cancellation branch can win before the send future is polled.
    pub async fn settle_cancelled(&mut self) -> Result<(), ControlledProviderError> {
        let outcome = if self.dispatch_started_ns.is_some() {
            EffectOutcome::Unknown
        } else {
            EffectOutcome::NotStarted
        };
        self.settle_active(outcome, None, false).await
    }

    /// Settles a send that did not return a successful response body stream.
    pub async fn settle_active(
        &mut self,
        outcome: EffectOutcome,
        reported_usage: Option<TokenUsage>,
        usage_complete: bool,
    ) -> Result<(), ControlledProviderError> {
        let Some((attempt, permit, dispatched)) = self.active.take() else {
            return Err(ControlledProviderError::OwnerFailure(
                EffectOwnerFailure::new(PhysicalAttemptContractViolation),
            ));
        };
        if (outcome == EffectOutcome::NotStarted) == dispatched
            || (reported_usage.is_none() && usage_complete)
        {
            return Err(self.contract_violation());
        }
        self.settle(permit, attempt, outcome, reported_usage, usage_complete)
            .await
    }

    /// Holds a successful HTTP response until the consumer knows its usage.
    pub fn hold_successful_response(&mut self) -> Result<(), ControlledProviderError> {
        let Some((_, _, dispatched)) = self.active.as_ref() else {
            return Err(ControlledProviderError::OwnerFailure(
                EffectOwnerFailure::new(PhysicalAttemptContractViolation),
            ));
        };
        if !dispatched {
            return Err(self.contract_violation());
        }
        let Some(active) = self.active.take() else {
            return Err(self.contract_violation());
        };
        if self
            .pending_response
            .replace((active.0, active.1))
            .is_some()
        {
            return Err(ControlledProviderError::OwnerFailure(
                EffectOwnerFailure::new(PhysicalAttemptContractViolation),
            ));
        }
        Ok(())
    }

    /// Settles a held response after its body has been consumed.
    pub async fn settle_pending_response(
        &mut self,
        outcome: EffectOutcome,
        reported_usage: Option<TokenUsage>,
    ) -> Result<(), ControlledProviderError> {
        let Some((attempt, permit)) = self.pending_response.take() else {
            return Ok(());
        };
        if outcome == EffectOutcome::NotStarted {
            return Err(self.contract_violation());
        }
        self.settle(
            permit,
            attempt,
            outcome,
            reported_usage,
            reported_usage.is_some(),
        )
        .await
    }

    pub(crate) const fn dispatched_any(&self) -> bool {
        self.dispatched_any
    }

    pub(crate) const fn usage_complete(&self) -> bool {
        self.usage_complete
    }

    async fn settle(
        &mut self,
        permit: EffectPermit,
        attempt: PhysicalAttempt,
        outcome: EffectOutcome,
        reported_usage: Option<TokenUsage>,
        usage_complete: bool,
    ) -> Result<(), ControlledProviderError> {
        let observation = super::physical_observation::PhysicalAttemptObservation::finish(
            self.dispatch_started_ns.take(),
            outcome,
            reported_usage,
            usage_complete,
        )?
        .with_correlation(self.request_correlation.clone())
        .with_request(self.request_metadata.as_ref());
        self.owner
            .observe_physical_attempt(permit, attempt, observation)
            .await
            .map_err(|error| {
                self.cancellation
                    .cancel_with_reason(CancellationReason::OwnerFailure);
                ControlledProviderError::OwnerFailure(error)
            })?;
        self.usage_complete &= usage_complete;
        self.owner
            .settle(
                permit,
                EffectSettlement::PhysicalAttempt {
                    attempt,
                    outcome,
                    reported_usage,
                    usage_complete,
                },
            )
            .await
            .map_err(|error| {
                self.cancellation
                    .cancel_with_reason(CancellationReason::OwnerFailure);
                ControlledProviderError::OwnerFailure(error)
            })
    }

    fn contract_violation(&self) -> ControlledProviderError {
        self.cancellation
            .cancel_with_reason(CancellationReason::OwnerFailure);
        ControlledProviderError::OwnerFailure(EffectOwnerFailure::new(
            PhysicalAttemptContractViolation,
        ))
    }

    fn refresh_request_correlation(&mut self) {
        if let Some(request) = self.request_metadata.as_mut() {
            request.set_correlation(self.request_correlation.clone());
        }
    }
}

#[derive(Debug)]
struct PhysicalAttemptContractViolation;

impl std::fmt::Display for PhysicalAttemptContractViolation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("physical attempt permit contract violated")
    }
}

impl Error for PhysicalAttemptContractViolation {}
