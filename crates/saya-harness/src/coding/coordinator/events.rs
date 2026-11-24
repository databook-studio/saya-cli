use super::super::child::ChildExecutionSpec;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentMetric, AnsweringObservation, BudgetDimension,
    CancellationReason, CancellationToken, ProviderContinuationPhase, ProviderRecoveryPhase,
    ProviderRecoveryReason, TokenUsage, ToolEffect, UsageCall,
};
use saya_types::{AgentId, InvocationId, JobId};
use serde::Serialize;
use std::{
    num::NonZeroU64,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use thiserror::Error;
use tokio::sync::mpsc;

pub const MAX_CHILD_EVENT_QUEUE_CAPACITY: usize = 32;
pub const MAX_CHILD_EVENT_BYTES: usize = 8 * 1024;

/// One content-free progress fact from the agent loop.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "progress", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ChildProgress {
    AnsweringBudgetStopped {
        dimension: BudgetDimension,
    },
    AnswerTextAvailable,
    TurnReset,
    TurnStarted,
    ProviderRecovery {
        phase: ProviderRecoveryPhase,
        reason: ProviderRecoveryReason,
        attempt: u8,
        limit: u8,
    },
    ProviderContinuation {
        phase: ProviderContinuationPhase,
        index: u32,
        limit: u32,
    },
    /// The provider cut this child's stream after it had delivered content, so
    /// the run continues without lowering the output ceiling. Carries the
    /// cut's own clock, the visible-byte estimate, and the configured ceiling
    /// the next request asks for.
    StreamCutRecovered {
        elapsed_secs: u32,
        estimated_tokens: u32,
        ceiling_after: u32,
    },
    ToolRequested {
        effect: Option<ToolEffect>,
    },
    ToolCompleted,
    ToolDenied,
    KnowledgeSupplied {
        dropped_by_bounds: u64,
    },
    KnowledgeProposed,
    KnowledgeLearningStarted,
    KnowledgeOverridden {
        finding_count: u64,
    },
    KnowledgeLearningSkipped,
    KnowledgeLearningDisabled {
        misses: u32,
    },
    ClarificationNeeded,
    Complete,
    Usage {
        call: UsageCall,
        usage: TokenUsage,
    },
    FailedAttemptUsage {
        usage: TokenUsage,
    },
    AnswerDesignated,
    ConsensusDecided {
        attempts: u64,
        voted: u64,
        votes: u64,
        margin: u64,
        tied: bool,
        probe_broke_tie: bool,
    },
}

/// Finite projection of the agent's source-owned local metrics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "metric", rename_all = "snake_case")]
pub enum ChildMetric {
    Phase {
        phase: &'static str,
        elapsed_ns: u64,
        outcome: &'static str,
    },
    Count {
        name: &'static str,
    },
    Useful {
        name: &'static str,
    },
    UsageCoverage {
        attempt_outcome: &'static str,
        reported: bool,
    },
}

/// Bounded in-memory progress/usage projection, not a durable or complete trace.
/// Identity always comes from the narrowed spec.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChildEvent {
    pub(super) job_id: JobId,
    pub(super) agent_id: AgentId,
    pub(super) invocation_id: InvocationId,
    pub(super) kind: ChildEventKind,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
/// A safe finite progress, source-owned observation, or numeric metric payload.
pub enum ChildEventKind {
    Completion {
        sequence: NonZeroU64,
    },
    ToolDiagnostic {
        diagnostic: saya_agent::ToolDiagnostic,
    },
    Compaction {
        observation: saya_agent::ContextCompaction,
    },
    Progress {
        progress: ChildProgress,
    },
    Observation {
        observation: AnsweringObservation,
    },
    Metric {
        metric: ChildMetric,
    },
}

impl ChildEvent {
    pub(super) fn completion(
        job_id: JobId,
        agent_id: AgentId,
        invocation_id: InvocationId,
        sequence: NonZeroU64,
    ) -> Self {
        Self {
            job_id,
            agent_id,
            invocation_id,
            kind: ChildEventKind::Completion { sequence },
        }
    }

    pub fn job_id(&self) -> &JobId {
        &self.job_id
    }

    pub fn agent_id(&self) -> &AgentId {
        &self.agent_id
    }

    pub fn invocation_id(&self) -> &InvocationId {
        &self.invocation_id
    }

    pub fn kind(&self) -> &ChildEventKind {
        &self.kind
    }

    pub fn to_json_bytes(&self) -> Result<Vec<u8>, ChildEventChannelError> {
        let bytes = serde_json::to_vec(self).map_err(|_| ChildEventChannelError::Malformed)?;
        if bytes.len() > MAX_CHILD_EVENT_BYTES {
            return Err(ChildEventChannelError::Oversize);
        }
        Ok(bytes)
    }
}

pub struct ChildEventSender {
    pub(super) sender: mpsc::Sender<ChildEvent>,
    pub(super) job_id: JobId,
    pub(super) agent_id: AgentId,
    pub(super) invocation_id: InvocationId,
    /// The parent run's own event stream, when the caller states one: this
    /// child's tool calls are forwarded into it, stamped with this child's id
    /// (`02` §3.6 groups a helper's work by its caller). `None` is a child
    /// whose caller keeps its stream to the root's own loop, which is every
    /// surface that does not draw a helper block.
    pub(super) forward: Option<Arc<dyn AgentEventSink>>,
}

pub struct ChildEventReceiver {
    receiver: mpsc::Receiver<ChildEvent>,
}

impl ChildEventReceiver {
    pub async fn recv(&mut self) -> Option<ChildEvent> {
        self.receiver.recv().await
    }

    pub fn capacity(&self) -> usize {
        self.receiver.capacity()
    }
}

pub fn child_event_channel(spec: &ChildExecutionSpec) -> (ChildEventSender, ChildEventReceiver) {
    let authority = spec.authority();
    let (sender, receiver) = mpsc::channel(MAX_CHILD_EVENT_QUEUE_CAPACITY);
    (
        ChildEventSender {
            sender,
            job_id: authority.job_id().clone(),
            agent_id: authority.agent_id().clone(),
            invocation_id: authority.invocation_id().clone(),
            forward: None,
        },
        ChildEventReceiver { receiver },
    )
}

impl ChildEventSender {
    /// The same sender, whose child also forwards its tool calls into `sink` —
    /// the parent run's stream — with `caller` set to this child's id.
    pub fn forwarding(mut self, sink: Arc<dyn AgentEventSink>) -> Self {
        self.forward = Some(sink);
        self
    }
}

/// Why a bounded child event could not be encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ChildEventChannelError {
    #[error("child event could not be encoded")]
    Malformed,
    #[error("child event exceeds the 8 KiB message bound")]
    Oversize,
}

pub(super) struct ChildEventSink {
    pub(super) events: ChildEventSender,
    pub(super) cancellation: CancellationToken,
    pub(super) owner_failure: AtomicBool,
    /// Where this child's tool calls go besides the progress channel: the
    /// parent run's own stream, when its caller stated one.
    pub(super) forward: Option<Arc<dyn AgentEventSink>>,
}

impl ChildEventSink {
    async fn publish(&self, kind: ChildEventKind) {
        let tool_diagnostic = matches!(&kind, ChildEventKind::ToolDiagnostic { .. });
        let event = ChildEvent {
            job_id: self.events.job_id.clone(),
            agent_id: self.events.agent_id.clone(),
            invocation_id: self.events.invocation_id.clone(),
            kind,
        };
        if event.to_json_bytes().is_err() {
            self.cancel_owner_failure();
            return;
        }
        tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => {
                // A budget stop can return successful truncated output. Its
                // known refusals still need bounded delivery; user cancellation
                // keeps the existing partial-trace semantics.
                if tool_diagnostic && matches!(self.cancellation.reason(), Some(CancellationReason::Budget(_))) {
                    if self.events.sender.try_send(event).is_err() {
                        self.cancel_owner_failure();
                    }
                } else if self.events.sender.is_closed() {
                    self.cancel_owner_failure();
                }
            }
            reserved = self.events.sender.reserve() => {
                match reserved {
                    Ok(permit) => { permit.send(event); }
                    Err(_) => self.cancel_owner_failure(),
                }
            }
        }
    }

    fn cancel_owner_failure(&self) {
        self.owner_failure.store(true, Ordering::Release);
        self.cancellation
            .cancel_with_reason(CancellationReason::OwnerFailure);
    }
}

#[async_trait::async_trait]
impl AgentEventSink for ChildEventSink {
    async fn observe_tool(&self, diagnostic: saya_agent::ToolDiagnostic) {
        self.publish(ChildEventKind::ToolDiagnostic { diagnostic })
            .await;
    }

    fn observe_compaction(&self, observation: saya_agent::ContextCompaction) {
        let event = ChildEvent {
            job_id: self.events.job_id.clone(),
            agent_id: self.events.agent_id.clone(),
            invocation_id: self.events.invocation_id.clone(),
            kind: ChildEventKind::Compaction { observation },
        };
        if event.to_json_bytes().is_err() || self.events.sender.try_send(event).is_err() {
            self.cancel_owner_failure();
        }
    }

    async fn emit(&self, event: AgentEvent) {
        // The parent run's stream: one tool call the helper made, stamped with
        // the helper's id so the transcript can group the work by its caller.
        // The forward is awaited inline and holds no queue of its own — the
        // bound is the parent sink's own backpressure, exactly what the root's
        // loop meets, so nothing accumulates here while a surface is slow.
        // Only the three tool events travel: the child's text, reasoning and
        // metrics stay its own, and a root's events never reach here.
        if let Some(forward) = &self.forward
            && let Some(call) = stamped(event.clone(), &self.events.agent_id)
        {
            forward.emit(call).await;
        }
        match super::event_projection::progress(event) {
            Ok(Some(progress)) => {
                self.publish(ChildEventKind::Progress { progress }).await;
            }
            Ok(None) => {}
            Err(()) => self.cancel_owner_failure(),
        }
    }

    async fn observe_answering(&self, observation: AnsweringObservation) {
        self.publish(ChildEventKind::Observation { observation })
            .await;
    }

    async fn metric(&self, metric: AgentMetric) {
        match super::event_projection::metric(metric) {
            Ok(metric) => self.publish(ChildEventKind::Metric { metric }).await,
            Err(()) => self.cancel_owner_failure(),
        }
    }
}

/// The tool events a child's loop emits, stamped with the child's own id — the
/// `caller` `02` §3.6 groups a helper's work by. Every other event is the
/// child's own and stays out of the parent's stream; a call that already names
/// a caller keeps it, so a helper of a helper is never relabelled as this one.
fn stamped(event: AgentEvent, agent_id: &AgentId) -> Option<AgentEvent> {
    let caller = agent_id.as_str().to_owned();
    Some(match event {
        AgentEvent::ToolRequested {
            name,
            arguments,
            effect,
            caller: None,
        } => AgentEvent::ToolRequested {
            name,
            arguments,
            effect,
            caller: Some(caller),
        },
        AgentEvent::ToolCompleted {
            name,
            summary,
            caller: None,
        } => AgentEvent::ToolCompleted {
            name,
            summary,
            caller: Some(caller),
        },
        AgentEvent::ToolDenied {
            name,
            reason,
            arguments,
            caller: None,
        } => AgentEvent::ToolDenied {
            name,
            reason,
            arguments,
            caller: Some(caller),
        },
        _ => return None,
    })
}
