//! The approval decider: consults the session policy, asks through the modal.

use super::super::messages::StreamMsg;
use crate::approval_facts::ApprovalFacts;
use crate::grant_token::{TurnPrimary, grant_token};
use crate::local_metrics::LocalMetrics;
use async_trait::async_trait;
use saya_agent::{
    AgentMetric, AgentMetricOutcome, AgentMetricPhase, ApprovalChoice, ApprovalDecider,
    ApprovalDecision, CancellationToken, SessionPolicy, ToolDefinition,
};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

/// Approval decider that consults the session policy: whatever the engine
/// allows or denies runs or refuses without the user; an ask is rendered as
/// the modal (over the same channel) and the user's [`ApprovalChoice`] is
/// recorded into the session policy and turned back into the decision. The
/// TUI can always prompt, so an ask never falls back to stdin. The policy is
/// the session's one instance, cloned per turn by `start` — a grant recorded
/// through one turn's ask is in force for every later turn.
pub(crate) struct ChannelApproval {
    tx: UnboundedSender<StreamMsg>,
    policy: SessionPolicy,
    /// The turn's primary connection, bound by the turn this decider
    /// belongs to; the SQL family's suggestion names it when the call
    /// names no connection.
    primary: TurnPrimary,
    /// The session composition's prompt facts: what this decider's modal
    /// may state. The same bundle the terminal decider renders, so the
    /// modal cannot state different facts.
    facts: ApprovalFacts,
    /// The session journal, when this decider belongs to a session: a
    /// `[s]` answer's new grant is journalled there, before the call it
    /// allowed runs. `None` — test shapes — records grants with no journal.
    journal: Option<Arc<saya_store::SessionJournal>>,
    /// The turn's metrics collector; `None` — the constructor default — records nothing.
    metrics: Option<Arc<LocalMetrics>>,
    cancellation: CancellationToken,
}

impl ChannelApproval {
    pub(crate) fn with_cancellation(
        tx: UnboundedSender<StreamMsg>,
        policy: SessionPolicy,
        primary: TurnPrimary,
        facts: ApprovalFacts,
        journal: Option<Arc<saya_store::SessionJournal>>,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            tx,
            policy,
            primary,
            facts,
            journal,
            metrics: None,
            cancellation,
        }
    }

    /// Attaches the turn's metrics collector; the constructor stays `None`.
    pub(crate) fn with_metrics(mut self, metrics: Option<Arc<LocalMetrics>>) -> Self {
        self.metrics = metrics;
        self
    }

    /// One ApprovalWait sample: the wait opens at the send — channel and UI
    /// scheduling included — and closes with the reply, cancel, or closure.
    fn record_wait(&self, started: Instant, outcome: AgentMetricOutcome) {
        let Some(metrics) = self.metrics.as_deref() else {
            return;
        };
        metrics.record(&AgentMetric::Phase {
            phase: AgentMetricPhase::ApprovalWait,
            elapsed_ns: u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX),
            outcome,
        });
    }
}

#[async_trait]
impl ApprovalDecider for ChannelApproval {
    async fn approve(&self, tool: &ToolDefinition, arguments: &serde_json::Value) -> bool {
        // Deny first, at every program-named door: a denied name refuses
        // (`false`) before grant lookup, before the modal, before bypass —
        // the loop then relays this decider's typed refusal (see
        // `refusal_detail` below), and the executor's own deny check stays
        // as the second door for any caller that executes without approving.
        if crate::interactive::session::permissions::deny::denied_call_program(
            &tool.name,
            arguments,
            &self.facts.denied_programs,
        )
        .is_some()
        {
            return false;
        }
        let primary = self.primary.get();
        let grant = grant_token(&tool.name, arguments, primary.as_deref(), &self.facts);
        match self.policy.resolve(&tool.effect, grant.as_deref()) {
            ApprovalDecision::Allow => true,
            ApprovalDecision::Deny { .. } => false,
            ApprovalDecision::Ask => {
                let (respond, answer) = oneshot::channel();
                // The detail is the shared fact body — the same bytes the
                // terminal prompt renders (`approval_facts::call_facts`) —
                // so the modal cannot state different facts for the same
                // call. `None` and the modal falls back to the tool's name.
                let detail = crate::approval_facts::call_facts(
                    &tool.name,
                    arguments,
                    grant.as_deref(),
                    &self.facts,
                    primary.as_deref(),
                    Some(self.policy.grants()),
                );
                let started = Instant::now();
                if self
                    .tx
                    .send(StreamMsg::ApprovalRequest {
                        tool: tool.name.clone(),
                        detail,
                        grant,
                        respond,
                    })
                    .is_err()
                {
                    self.record_wait(started, AgentMetricOutcome::Failed);
                    return false;
                }
                tokio::select! {
                    _ = self.cancellation.cancelled() => {
                        self.record_wait(started, AgentMetricOutcome::Cancelled);
                        false
                    }
                    result = answer => match result {
                    Ok(choice) => {
                        self.record_wait(started, AgentMetricOutcome::Succeeded);
                        // The user's answer reaches the session's grant store
                        // here, where the policy lives; allow-once and deny
                        // record nothing. A *new* grant is journalled right
                        // here — before this call is allowed to run — by the
                        // shared operation, one wording. A failed journal
                        // write changes no consent: it is said into the
                        // transcript, never silent.
                        let (_, warning) = crate::interactive::session::permissions::grants::record_prompt_answer(
                            &self.policy,
                            &choice,
                            self.journal.as_deref(),
                        );
                        if let Some(warning) = warning {
                            let _ = self.tx.send(StreamMsg::Notice(warning));
                        }
                        !matches!(choice, ApprovalChoice::Deny)
                    }
                    // A UI that died mid-ask denies.
                    Err(_) => {
                        self.record_wait(started, AgentMetricOutcome::Failed);
                        false
                    }
                    }
                }
            }
        }
    }

    fn refusal_detail(
        &self,
        tool: &ToolDefinition,
        arguments: &serde_json::Value,
    ) -> Option<String> {
        // The loop reads this only after `approve` denied, so the same pure
        // seam answers the wording: the typed refusal for a denied program,
        // `None` for every denial this decider did not word (mode denials,
        // the modal's deny, a dead UI), where the loop's generic denial stands.
        crate::interactive::session::permissions::deny::denied_call_program(
            &tool.name,
            arguments,
            &self.facts.denied_programs,
        )
        .map(|program| crate::interactive::session::permissions::deny::denied_refusal(&program))
    }
}
