use std::{
    collections::VecDeque,
    num::NonZeroU64,
    sync::{
        Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use saya_agent::{
    AgentRunControl, EffectAdmission, EffectAdmissionError, EffectCallKind, EffectOwnerFailure,
    EffectPermit, EffectSettlement, PhysicalAttempt, ToolEffect,
};

#[derive(Default)]
pub struct Records {
    pub models: Mutex<Vec<EffectPermit>>,
    pub physical: Mutex<Vec<(EffectPermit, PhysicalAttempt)>>,
    pub settlements: Mutex<Vec<(EffectPermit, EffectSettlement)>>,
    next: AtomicU64,
    pub attempts_seen: AtomicUsize,
    pub denied: AtomicUsize,
    pub fail_settlement: AtomicUsize,
    pub cancel_after_admission: Mutex<Option<saya_agent::CancellationToken>>,
}

impl Records {
    pub fn permit(&self) -> EffectPermit {
        EffectPermit::new(NonZeroU64::new(self.next.fetch_add(1, Ordering::SeqCst) + 1).unwrap())
    }

    pub fn strict_control<'a>(
        &'a self,
        approval: &'a saya_agent::AllowReadOnlyApproval,
        cancellation: saya_agent::CancellationToken,
    ) -> AgentRunControl<'a> {
        self.strict_control_with_limits(approval, cancellation, saya_agent::AgentLimits::default())
    }

    pub fn strict_control_with_limits<'a>(
        &'a self,
        approval: &'a saya_agent::AllowReadOnlyApproval,
        cancellation: saya_agent::CancellationToken,
        limits: saya_agent::AgentLimits,
    ) -> AgentRunControl<'a> {
        AgentRunControl::new(
            limits,
            saya_agent::AnsweringLimits {
                max_requests: Some(8),
                max_tool_calls: Some(8),
                max_reported_tokens: Some(10_000),
            },
            approval,
            &saya_agent::NoopEventSink,
            cancellation,
        )
        .with_physical_effect_admission(self)
    }

    pub fn request() -> saya_agent::AgentRequest {
        saya_agent::AgentRequest {
            prompt: "answer locally".to_owned(),
            profile_names: Vec::new(),
            model: "test-model".to_owned(),
            system_prompt: None,
            history: Vec::new(),
            context_blocks: Vec::new(),
        }
    }
}

#[async_trait]
impl EffectAdmission for Records {
    async fn admit_model_request(&self) -> Result<EffectPermit, EffectAdmissionError> {
        let permit = self.permit();
        self.models.lock().unwrap().push(permit);
        Ok(permit)
    }

    async fn admit_tool_batch(
        &self,
        effects: &[ToolEffect],
    ) -> Result<Vec<EffectPermit>, EffectAdmissionError> {
        Ok(effects.iter().map(|_| self.permit()).collect())
    }

    async fn admit_physical_attempt(
        &self,
        attempt: PhysicalAttempt,
    ) -> Result<EffectPermit, EffectAdmissionError> {
        self.attempts_seen.fetch_add(1, Ordering::SeqCst);
        if self.denied.load(Ordering::SeqCst) > 0 {
            return Err(EffectAdmissionError::Denied);
        }
        let permit = self.permit();
        self.physical.lock().unwrap().push((permit, attempt));
        if let Some(token) = self.cancel_after_admission.lock().unwrap().take() {
            token.cancel();
        }
        Ok(permit)
    }

    async fn settle(
        &self,
        permit: EffectPermit,
        settlement: EffectSettlement,
    ) -> Result<(), EffectOwnerFailure> {
        self.settlements.lock().unwrap().push((permit, settlement));
        if self.fail_settlement.load(Ordering::SeqCst) > 0 {
            return Err(EffectOwnerFailure::new(std::io::Error::other(
                "physical settlement failed",
            )));
        }
        Ok(())
    }
}

pub fn call_kinds(attempts: &[(EffectPermit, PhysicalAttempt)]) -> Vec<EffectCallKind> {
    attempts.iter().map(|(_, attempt)| attempt.kind()).collect()
}

pub struct NoTools;

#[async_trait]
impl saya_agent::ToolExecutor for NoTools {
    async fn execute(
        &self,
        _: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, saya_agent::ToolError> {
        unreachable!("the fixed provider response has no tool calls")
    }
}

pub struct ControlledScriptProvider {
    pub scripts: Mutex<VecDeque<Vec<Result<saya_agent::ProviderEvent, saya_agent::ProviderError>>>>,
    pub old_calls: AtomicUsize,
}

impl ControlledScriptProvider {
    pub fn new(
        scripts: Vec<Vec<Result<saya_agent::ProviderEvent, saya_agent::ProviderError>>>,
    ) -> Self {
        Self {
            scripts: Mutex::new(scripts.into()),
            old_calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl saya_agent::ChatProvider for ControlledScriptProvider {
    fn name(&self) -> &str {
        "n02-controlled-script"
    }

    async fn complete(
        &self,
        _: saya_agent::ChatRequest,
    ) -> Result<saya_agent::ChatResponse, saya_agent::ProviderError> {
        unreachable!("the run uses streaming")
    }

    async fn stream(
        &self,
        _: saya_agent::ChatRequest,
        _: saya_agent::CancellationToken,
    ) -> Result<saya_agent::ProviderStream, saya_agent::ProviderError> {
        self.old_calls.fetch_add(1, Ordering::SeqCst);
        Err(saya_agent::ProviderError::InvalidResponse)
    }

    async fn stream_with_physical_attempts(
        &self,
        _: saya_agent::ChatRequest,
        _: saya_agent::CancellationToken,
        attempts: &mut saya_agent::PhysicalAttemptControl<'_>,
    ) -> Result<saya_agent::ProviderStream, saya_agent::ControlledProviderError> {
        attempts.admit_before_send().await?;
        attempts.dispatch_started()?;
        attempts.hold_successful_response()?;
        let script = self.scripts.lock().unwrap().pop_front().unwrap_or_else(|| {
            vec![
                Ok(saya_agent::ProviderEvent::TextDelta("answer".into())),
                Ok(saya_agent::ProviderEvent::Done),
            ]
        });
        Ok(Box::pin(futures_util::stream::iter(script)))
    }
}

pub fn successful_script() -> Vec<Result<saya_agent::ProviderEvent, saya_agent::ProviderError>> {
    vec![
        Ok(saya_agent::ProviderEvent::TextDelta("answer".into())),
        Ok(saya_agent::ProviderEvent::Done),
    ]
}
