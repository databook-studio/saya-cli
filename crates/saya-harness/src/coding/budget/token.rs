//! Optional strict aggregate allowance within the existing budget owner.
use super::ledger::{JobBudgetLedger, State};
use saya_agent::{
    EffectAdmissionError, EffectOutcome, EffectOwnerFailure, PhysicalRequestMetadata, TokenUsage,
};
use saya_types::{BudgetDimension, BudgetObservation, BudgetObservationValue, TokenBudgetPolicyV1};
impl JobBudgetLedger {
    pub async fn configure_token_budget(
        &self,
        policy: TokenBudgetPolicyV1,
    ) -> Result<(), saya_store::StoreError> {
        let mut state = self.inner.lock().await;
        if state.quiescent.load(std::sync::atomic::Ordering::Acquire) {
            return Err(saya_store::StoreError::Conflict);
        }
        state
            .store
            .record_coding_token_policy(&state.job, &policy)
            .await?;
        state.token_policy = Some(policy);
        Ok(())
    }
}
pub(super) fn reserve_amount(
    state: &State,
    request: Option<&PhysicalRequestMetadata>,
) -> Result<Option<(u64, u64)>, EffectAdmissionError> {
    let Some(policy) = &state.token_policy else {
        return Ok(None);
    };
    let request = request.ok_or(EffectAdmissionError::Denied)?;
    if !policy.matches_request(
        request.provider(),
        request.model(),
        request.endpoint(),
        request.output_tokens(),
    ) {
        return Err(EffectAdmissionError::Denied);
    }
    let data = policy.data();
    let output = u64::from(data.output_tokens);
    let required = data
        .input_context_upper_bound
        .checked_add(output)
        .ok_or(EffectAdmissionError::Denied)?;
    if state
        .counts
        .tokens
        .checked_add(required)
        .is_none_or(|total| total > data.total_tokens)
    {
        return Err(EffectAdmissionError::Denied);
    }
    Ok(Some((data.input_context_upper_bound, output)))
}
pub(super) fn observations(
    state: &State,
    outcome: EffectOutcome,
    usage: Option<TokenUsage>,
    complete: bool,
) -> Vec<BudgetObservation> {
    if state.token_policy.is_none() {
        return Vec::new();
    }
    let values = if outcome == EffectOutcome::NotStarted {
        Some((0, 0))
    } else if outcome == EffectOutcome::Succeeded && complete {
        usage.map(|usage| (usage.input_tokens, usage.output_tokens))
    } else {
        None
    };
    [
        (BudgetDimension::InputTokens, values.map(|v| v.0)),
        (BudgetDimension::OutputTokens, values.map(|v| v.1)),
    ]
    .into_iter()
    .map(|(dimension, value)| {
        BudgetObservation::new(
            dimension,
            None,
            None,
            value
                .map(BudgetObservationValue::Measured)
                .unwrap_or(BudgetObservationValue::Unknown),
        )
        .expect("global token observation shape is valid")
    })
    .collect()
}
pub(super) fn settle(
    state: &mut State,
    outcome: EffectOutcome,
    usage: Option<TokenUsage>,
    complete: bool,
) -> Result<(), EffectOwnerFailure> {
    let Some(policy) = &state.token_policy else {
        return Ok(());
    };
    let data = policy.data();
    let reserved = data.input_context_upper_bound + u64::from(data.output_tokens);
    let violated = usage.is_some_and(|usage| {
        usage.input_tokens > data.input_context_upper_bound
            || usage.output_tokens > u64::from(data.output_tokens)
    });
    let measured = if outcome == EffectOutcome::NotStarted {
        Some(0)
    } else if outcome == EffectOutcome::Succeeded && complete {
        usage.and_then(|usage| usage.input_tokens.checked_add(usage.output_tokens))
    } else {
        None
    };
    if let Some(measured) = measured {
        state.counts.tokens = state
            .counts
            .tokens
            .checked_sub(reserved)
            .and_then(|held| held.checked_add(measured))
            .ok_or_else(|| super::accounting::owner_error(state))?;
    }
    if violated
        || (outcome == EffectOutcome::Succeeded
            && complete
            && usage.is_some()
            && measured.is_none())
    {
        state.snapshot.token_bound_violated = true;
        return Err(super::accounting::owner_error(state));
    }
    Ok(())
}
