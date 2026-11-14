use std::{
    num::NonZeroU64,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use async_trait::async_trait;
use saya_agent::{
    EffectAdmission, EffectAdmissionError, EffectOutcome, EffectOwnerFailure, EffectPermit,
    EffectSettlement, TokenUsage, ToolEffect,
};

pub type Trace = Arc<Mutex<Vec<String>>>;

pub fn trace() -> Trace {
    Arc::new(Mutex::new(Vec::new()))
}

#[derive(Default)]
pub struct OwnerOptions {
    pub deny_model: bool,
    pub deny_external_tool: bool,
    pub short_tool_batch: bool,
    pub fail_settlement: bool,
    pub cancel_on_model_admission: Option<saya_agent::CancellationToken>,
    pub cancel_on_tool_admission: Option<saya_agent::CancellationToken>,
    pub deny_model_after: Option<usize>,
    pub fail_model_after: Option<usize>,
    pub cancel_before_model_failure: Option<saya_agent::CancellationToken>,
}

pub struct RecordingOwner {
    pub trace: Trace,
    next: AtomicU64,
    model_admissions: std::sync::atomic::AtomicUsize,
    pub settlements: Mutex<Vec<(u64, EffectSettlement)>>,
    options: OwnerOptions,
}

impl RecordingOwner {
    pub fn new(trace: Trace, options: OwnerOptions) -> Self {
        Self {
            trace,
            next: AtomicU64::new(1),
            model_admissions: std::sync::atomic::AtomicUsize::new(0),
            settlements: Mutex::new(Vec::new()),
            options,
        }
    }

    fn permit(&self) -> EffectPermit {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        EffectPermit::new(NonZeroU64::new(id).unwrap())
    }
}

#[async_trait]
impl EffectAdmission for RecordingOwner {
    async fn admit_model_request(&self) -> Result<EffectPermit, EffectAdmissionError> {
        let index = self.model_admissions.fetch_add(1, Ordering::SeqCst);
        self.trace.lock().unwrap().push("admit:model".to_owned());
        if self.options.deny_model || self.options.deny_model_after == Some(index) {
            return Err(EffectAdmissionError::Denied);
        }
        if self.options.fail_model_after == Some(index) {
            if let Some(token) = &self.options.cancel_before_model_failure {
                token.cancel();
            }
            return Err(EffectAdmissionError::OwnerFailure(EffectOwnerFailure::new(
                std::io::Error::other("PRIVATE-MODEL-ADMISSION-FAILURE"),
            )));
        }
        if let Some(token) = &self.options.cancel_on_model_admission {
            token.cancel();
        }
        Ok(self.permit())
    }

    async fn admit_tool_batch(
        &self,
        effects: &[ToolEffect],
    ) -> Result<Vec<EffectPermit>, EffectAdmissionError> {
        self.trace
            .lock()
            .unwrap()
            .push(format!("admit:tools:{}", effects.len()));
        if self.options.deny_external_tool
            && effects.iter().any(|effect| effect.external_side_effect)
        {
            return Err(EffectAdmissionError::Denied);
        }
        if let Some(token) = &self.options.cancel_on_tool_admission {
            token.cancel();
        }
        let count = if self.options.short_tool_batch {
            effects.len().saturating_sub(1)
        } else {
            effects.len()
        };
        Ok((0..count).map(|_| self.permit()).collect())
    }

    async fn settle(
        &self,
        permit: EffectPermit,
        settlement: EffectSettlement,
    ) -> Result<(), EffectOwnerFailure> {
        self.trace
            .lock()
            .unwrap()
            .push(format!("settle:{}", permit.get()));
        self.settlements
            .lock()
            .unwrap()
            .push((permit.get(), settlement));
        if self.options.fail_settlement {
            return Err(EffectOwnerFailure::new(std::io::Error::other(
                "PRIVATE-OWNER-FAILURE-SENTINEL",
            )));
        }
        Ok(())
    }
}

pub fn model(
    outcome: EffectOutcome,
    reported_usage: Option<TokenUsage>,
    usage_complete: bool,
) -> EffectSettlement {
    EffectSettlement::ModelRequest {
        outcome,
        reported_usage,
        usage_complete,
    }
}

pub fn tool(outcome: EffectOutcome) -> EffectSettlement {
    EffectSettlement::Tool { outcome }
}
