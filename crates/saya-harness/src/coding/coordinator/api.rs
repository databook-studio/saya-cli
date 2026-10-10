use saya_agent::{
    AgentError, AgentEvent, AgentEventSink, AgentMetric, AgentRunControl, AnsweringObservation,
    BudgetDimension, CancellationReason, CancellationToken, ProviderContinuationPhase,
    ProviderRecoveryPhase, ProviderRecoveryReason, TokenUsage, ToolEffect, UsageCall,
    run_agent_with_control,
};
use saya_types::{AgentId, InvocationId, JobId};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use thiserror::Error;
use tokio::sync::mpsc;

use super::super::child::{ChildComponents, ChildExecutionSpec, ChildFactory, ChildFactoryError};

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
    job_id: JobId,
    agent_id: AgentId,
    invocation_id: InvocationId,
    kind: ChildEventKind,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
/// A safe finite progress, source-owned observation, or numeric metric payload.
pub enum ChildEventKind {
    Progress { progress: ChildProgress },
    Observation { observation: AnsweringObservation },
    Metric { metric: ChildMetric },
}

impl ChildEvent {
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
    sender: mpsc::Sender<ChildEvent>,
    job_id: JobId,
    agent_id: AgentId,
    invocation_id: InvocationId,
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
        },
        ChildEventReceiver { receiver },
    )
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

/// Why the one child invocation did not produce its ordinary result.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ChildRunError {
    #[error(transparent)]
    Factory(#[from] ChildFactoryError),
    #[error("child event channel identity does not match its execution spec")]
    ChannelIdentityMismatch,
    #[error("child progress owner failed")]
    OwnerFailure,
    #[error("child invocation was cancelled: {reason:?}")]
    Cancelled { reason: CancellationReason },
    #[error(transparent)]
    Agent(#[from] AgentError),
}

/// Run one child through the existing agent loop; scheduling and joining stay with the caller.
pub async fn run_child(
    spec: ChildExecutionSpec,
    factory: &dyn ChildFactory,
    events: ChildEventSender,
    cancellation: CancellationToken,
) -> Result<saya_agent::AgentOutput, ChildRunError> {
    let authority = spec.authority();
    if &events.job_id != authority.job_id()
        || &events.agent_id != authority.agent_id()
        || &events.invocation_id != authority.invocation_id()
    {
        return Err(ChildRunError::ChannelIdentityMismatch);
    }
    let ChildComponents {
        provider,
        tools,
        definitions,
        approval,
    } = factory.build(&spec)?;
    let sink = ChildEventSink {
        events,
        cancellation: cancellation.clone(),
        owner_failure: AtomicBool::new(false),
    };
    let (request, limits, answering, preflight) = spec.into_run_parts();
    let control = AgentRunControl::new(
        limits,
        answering,
        approval.as_ref(),
        &sink,
        cancellation.clone(),
    );
    let control = match preflight {
        Some(config) => control.with_request_preflight(config),
        None => control,
    };
    let result = run_agent_with_control(
        provider.as_ref(),
        tools.as_ref(),
        request,
        definitions,
        control,
    )
    .await;
    let reason = cancellation.reason();
    if sink.owner_failure.load(Ordering::Acquire)
        || reason == Some(CancellationReason::OwnerFailure)
    {
        return Err(ChildRunError::OwnerFailure);
    }
    if result.is_ok() && reason == Some(CancellationReason::User) {
        return Err(ChildRunError::Cancelled {
            reason: CancellationReason::User,
        });
    }
    if matches!(&result, Err(AgentError::Cancelled)) {
        if let Some(reason) = reason {
            return Err(ChildRunError::Cancelled { reason });
        }
    }
    result.map_err(ChildRunError::Agent)
}

struct ChildEventSink {
    events: ChildEventSender,
    cancellation: CancellationToken,
    owner_failure: AtomicBool,
}

impl ChildEventSink {
    async fn publish(&self, kind: ChildEventKind) {
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
                if self.events.sender.is_closed() {
                    self.cancel_owner_failure();
                }
            }
            sent = self.events.sender.send(event) => {
                if sent.is_err() {
                    self.cancel_owner_failure();
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
    async fn emit(&self, event: AgentEvent) {
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
