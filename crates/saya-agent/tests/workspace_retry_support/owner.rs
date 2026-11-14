use async_trait::async_trait;
use saya_agent::{
    EffectAdmission, EffectAdmissionError, EffectOutcome, EffectOwnerFailure, EffectPermit,
    EffectSettlement, ToolEffect,
};
use std::{
    num::NonZeroU64,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Default)]
pub struct Owner {
    next: AtomicU64,
    pub tools: Mutex<Vec<EffectOutcome>>,
    pub fail_write_settlement: bool,
}
impl Owner {
    fn permit(&self) -> EffectPermit {
        EffectPermit::new(NonZeroU64::new(self.next.fetch_add(1, Ordering::AcqRel) + 1).unwrap())
    }
}
#[async_trait]
impl EffectAdmission for Owner {
    async fn admit_model_request(&self) -> Result<EffectPermit, EffectAdmissionError> {
        Ok(self.permit())
    }
    async fn admit_tool_batch(
        &self,
        effects: &[ToolEffect],
    ) -> Result<Vec<EffectPermit>, EffectAdmissionError> {
        Ok(effects.iter().map(|_| self.permit()).collect())
    }
    async fn settle(
        &self,
        _: EffectPermit,
        settlement: EffectSettlement,
    ) -> Result<(), EffectOwnerFailure> {
        if let EffectSettlement::Tool { outcome } = settlement {
            let mut tools = self.tools.lock().unwrap();
            tools.push(outcome);
            if self.fail_write_settlement && tools.len() == 2 {
                return Err(EffectOwnerFailure::new(std::io::Error::other(
                    "synthetic authoritative persistence failure",
                )));
            }
        }
        Ok(())
    }
}
