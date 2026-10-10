//! Command effect budget lifecycle for the coding ledger. Commands bypass
//! the agent loop like child sessions: `begin_command`/`finish_command`
//! live outside the `EffectAdmission` trait. A `CommandPermit` dropped
//! without settlement stays held as Unknown by design (the saya-agent
//! `coding_admission.rs` contract).

use super::ledger::JobBudgetLedger;
use saya_agent::{EffectAdmissionError, EffectOutcome, EffectPermit};
use saya_types::BudgetDimension;

/// An admitted command slot bound to the ledger that admitted it: settling it on any other ledger is an owner failure, as for child sessions.
#[derive(Clone)]
pub struct CommandPermit {
    permit: EffectPermit,
    owner: std::sync::Arc<tokio::sync::Mutex<super::ledger::State>>,
}

impl JobBudgetLedger {
    pub async fn begin_command(&self) -> Result<CommandPermit, EffectAdmissionError> {
        let mut state = self.inner.lock().await;
        let permit = state
            .admit_one(BudgetDimension::CommandSlots, 1, None, None)
            .await?;
        Ok(CommandPermit {
            permit,
            owner: self.inner.clone(),
        })
    }

    pub async fn finish_command(
        &self,
        permit: CommandPermit,
        outcome: EffectOutcome,
    ) -> Result<(), EffectAdmissionError> {
        let mut state = self.inner.lock().await;
        if !std::sync::Arc::ptr_eq(&permit.owner, &self.inner) {
            return Err(state.owner_failure());
        }
        state
            .finish(
                permit.permit,
                super::settlement::Settlement::Command(outcome),
            )
            .await?;
        if outcome != EffectOutcome::Unknown {
            state.counts.command_slots = state
                .counts
                .command_slots
                .checked_sub(1)
                .ok_or_else(|| state.owner_failure())?;
        }
        Ok(())
    }
}
