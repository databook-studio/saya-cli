//! The owner's shared interior state, its counters, and the child-session
//! handle built on it.

use saya_agent::{CancellationReason, EffectAdmissionError, EffectOwnerFailure, EffectPermit};
use saya_store::SqliteStateStore;
use saya_types::{AgentId, BudgetDimension, InvocationId, JobId};
use std::{
    collections::HashMap,
    num::NonZeroU64,
    sync::Arc,
    time::{Duration, Instant},
};

use super::super::admission::StoredPermit;

pub(in crate::coding::budget) struct State {
    pub(in crate::coding::budget) token_policy: Option<saya_types::TokenBudgetPolicyV1>,
    pub(in crate::coding::budget) quiescent: Arc<std::sync::atomic::AtomicBool>,
    pub(in crate::coding::budget) runtime_authority: Option<saya_store::CodingRuntimeAuthority>,
    pub(in crate::coding::budget) runtime_renewal_required: bool,
    pub(in crate::coding::budget) store: SqliteStateStore,
    pub(in crate::coding::budget) job: JobId,
    pub(in crate::coding::budget) limits: super::CountLimits,
    pub(in crate::coding::budget) started: Instant,
    pub(in crate::coding::budget) sequence: u64,
    pub(in crate::coding::budget) next_id: u64,
    pub(in crate::coding::budget) reservations: u64,
    pub(in crate::coding::budget) counts: Counts,
    pub(in crate::coding::budget) permits: HashMap<u64, StoredPermit>,
    pub(in crate::coding::budget) groups: HashMap<NonZeroU64, super::super::tools::ToolGroup>,
    pub(in crate::coding::budget) local: HashMap<(AgentId, InvocationId), LocalCounts>,
    pub(in crate::coding::budget) cancellation: saya_agent::CancellationToken,
    pub(in crate::coding::budget) snapshot: super::BudgetSnapshot,
    /// One usage observation was unknown or lacked complete reported totals;
    /// usage coverage may never read Complete again.
    pub(in crate::coding::budget) usage_ever_unknown: bool,
    pub(in crate::coding::budget) usage_ever_incomplete: bool,
}

#[derive(Default)]
pub(in crate::coding::budget) struct Counts {
    pub(in crate::coding::budget) tokens: u64,
    pub(in crate::coding::budget) logical: u64,
    pub(in crate::coding::budget) transport: u64,
    pub(in crate::coding::budget) tools: u64,
    pub(in crate::coding::budget) model_slots: u64,
    pub(in crate::coding::budget) child_slots: u64,
    pub(in crate::coding::budget) command_slots: u64,
}

#[derive(Default)]
pub(in crate::coding::budget) struct LocalCounts {
    pub(in crate::coding::budget) logical: u64,
    pub(in crate::coding::budget) tools: u64,
    pub(in crate::coding::budget) limits: Option<super::ChildCountLimits>,
}

impl State {
    pub(in crate::coding::budget) fn remaining(&self) -> Duration {
        self.limits.deadline.saturating_sub(self.started.elapsed())
    }

    pub(in crate::coding::budget) fn owner_failure(&mut self) -> EffectAdmissionError {
        self.snapshot.poisoned = true;
        self.cancellation
            .cancel_with_reason(CancellationReason::OwnerFailure);
        EffectOwnerFailure::new(std::io::Error::other("budget owner invariant failed")).into()
    }

    pub(in crate::coding::budget) fn deadline_expired(&self) -> bool {
        self.remaining().is_zero()
    }

    /// Latches the root deadline reason without overwriting an earlier cause.
    pub(in crate::coding::budget) fn latch_deadline_expiry(&self) {
        self.cancellation
            .cancel_with_reason(CancellationReason::Budget(
                saya_agent::BudgetDimension::ElapsedTime,
            ));
    }

    pub(in crate::coding::budget) async fn admit_one(
        &mut self,
        dimension: BudgetDimension,
        amount: u64,
        agent: Option<AgentId>,
        invocation: Option<InvocationId>,
    ) -> Result<EffectPermit, EffectAdmissionError> {
        super::super::admission::admit_one(self, dimension, amount, agent, invocation).await
    }

    pub(in crate::coding::budget) async fn finish(
        &mut self,
        permit: EffectPermit,
        settlement: super::super::settlement::Settlement,
    ) -> Result<(), EffectAdmissionError> {
        super::super::settlement::finish(self, permit, settlement).await
    }
}

pub struct ChildSession {
    pub(in crate::coding::budget) permit: EffectPermit,
    pub(in crate::coding::budget) view: super::super::ChildBudgetView,
}

impl ChildSession {
    pub fn view(&self) -> &super::super::ChildBudgetView {
        &self.view
    }
}
