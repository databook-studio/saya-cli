use saya_agent::{
    AgentAttemptOutcome, AgentEvent, AgentMetric, AgentMetricCount, AgentMetricOutcome,
    AgentMetricPhase, AgentMetricUseful,
};

use super::events::{ChildMetric, ChildProgress};

pub(super) fn progress(event: AgentEvent) -> Result<Option<ChildProgress>, ()> {
    Ok(Some(match event {
        AgentEvent::AnsweringBudgetStopped { dimension } => {
            ChildProgress::AnsweringBudgetStopped { dimension }
        }
        AgentEvent::AssistantText { .. } => ChildProgress::AnswerTextAvailable,
        // Reasoning never enters the child progress channel.
        AgentEvent::ReasoningText { .. } => return Ok(None),
        AgentEvent::TurnReset => ChildProgress::TurnReset,
        AgentEvent::TurnStarted => ChildProgress::TurnStarted,
        AgentEvent::ProviderRecovery {
            phase,
            reason,
            attempt,
            limit,
        } => ChildProgress::ProviderRecovery {
            phase,
            reason,
            attempt,
            limit,
        },
        AgentEvent::ProviderContinuation {
            phase,
            index,
            limit,
        } => ChildProgress::ProviderContinuation {
            phase,
            index,
            limit,
        },
        AgentEvent::ToolRequested { effect, .. } => ChildProgress::ToolRequested { effect },
        AgentEvent::ToolCompleted { .. } => ChildProgress::ToolCompleted,
        AgentEvent::ToolDenied { .. } => ChildProgress::ToolDenied,
        AgentEvent::KnowledgeSupplied {
            dropped_by_bounds, ..
        } => ChildProgress::KnowledgeSupplied {
            dropped_by_bounds: u64::try_from(dropped_by_bounds).map_err(|_| ())?,
        },
        AgentEvent::KnowledgeProposed { .. } => ChildProgress::KnowledgeProposed,
        AgentEvent::KnowledgeLearningStarted => ChildProgress::KnowledgeLearningStarted,
        AgentEvent::KnowledgeOverridden { findings } => ChildProgress::KnowledgeOverridden {
            finding_count: u64::try_from(findings.len()).map_err(|_| ())?,
        },
        AgentEvent::KnowledgeLearningSkipped { .. } => ChildProgress::KnowledgeLearningSkipped,
        AgentEvent::KnowledgeLearningDisabled { misses, .. } => {
            ChildProgress::KnowledgeLearningDisabled { misses }
        }
        AgentEvent::ClarificationNeeded { .. } => ChildProgress::ClarificationNeeded,
        AgentEvent::Complete => ChildProgress::Complete,
        AgentEvent::Usage { call, usage } => ChildProgress::Usage { call, usage },
        AgentEvent::FailedAttemptUsage { usage } => ChildProgress::FailedAttemptUsage { usage },
        AgentEvent::AnswerDesignated { .. } => ChildProgress::AnswerDesignated,
        AgentEvent::ConsensusDecided {
            attempts,
            voted,
            votes,
            margin,
            tied,
            probe_broke_tie,
            ..
        } => ChildProgress::ConsensusDecided {
            attempts: u64::try_from(attempts).map_err(|_| ())?,
            voted: u64::try_from(voted).map_err(|_| ())?,
            votes: u64::try_from(votes).map_err(|_| ())?,
            margin: u64::try_from(margin).map_err(|_| ())?,
            tied,
            probe_broke_tie,
        },
        _ => return Err(()),
    }))
}

pub(super) fn metric(metric: AgentMetric) -> Result<ChildMetric, ()> {
    Ok(match metric {
        AgentMetric::Phase {
            phase,
            elapsed_ns,
            outcome,
        } => ChildMetric::Phase {
            phase: phase_name(phase).ok_or(())?,
            elapsed_ns,
            outcome: outcome_name(outcome).ok_or(())?,
        },
        AgentMetric::Count(count) => ChildMetric::Count {
            name: count_name(count).ok_or(())?,
        },
        AgentMetric::Useful(useful) => ChildMetric::Useful {
            name: useful_name(useful).ok_or(())?,
        },
        AgentMetric::UsageCoverage {
            attempt_outcome,
            reported,
        } => ChildMetric::UsageCoverage {
            attempt_outcome: attempt_name(attempt_outcome).ok_or(())?,
            reported,
        },
        _ => return Err(()),
    })
}

fn phase_name(phase: AgentMetricPhase) -> Option<&'static str> {
    Some(match phase {
        AgentMetricPhase::ProviderEstablishment => "provider_establishment",
        AgentMetricPhase::ProviderReadAwait => "provider_read_await",
        AgentMetricPhase::FirstProviderEvent => "first_provider_event",
        AgentMetricPhase::LocalAttempt => "local_attempt",
        AgentMetricPhase::RetryBackoff => "retry_backoff",
        AgentMetricPhase::DatabaseConnect => "database_connect",
        AgentMetricPhase::DatabaseWait => "database_wait",
        AgentMetricPhase::DatabaseExecute => "database_execute",
        AgentMetricPhase::SchemaDiscovery => "schema_discovery",
        AgentMetricPhase::ApprovalWait => "approval_wait",
        AgentMetricPhase::Compaction => "compaction",
        AgentMetricPhase::FirstPaint => "first_paint",
        AgentMetricPhase::QuestionFirstUseful => "question_first_useful",
        AgentMetricPhase::CancellationToLocalSettlement => "cancellation_to_local_settlement",
        _ => return None,
    })
}

fn outcome_name(outcome: AgentMetricOutcome) -> Option<&'static str> {
    Some(match outcome {
        AgentMetricOutcome::Succeeded => "succeeded",
        AgentMetricOutcome::Failed => "failed",
        AgentMetricOutcome::Cancelled => "cancelled",
        AgentMetricOutcome::Unavailable => "unavailable",
        AgentMetricOutcome::CachedFallback => "cached_fallback",
        AgentMetricOutcome::Skipped => "skipped",
        _ => return None,
    })
}

fn count_name(count: AgentMetricCount) -> Option<&'static str> {
    Some(match count {
        AgentMetricCount::RetryAttemptStarted => "retry_attempt_started",
        AgentMetricCount::OutputTruncated => "output_truncated",
        AgentMetricCount::AdmittedContinuation => "admitted_continuation",
        AgentMetricCount::CompactionStarted => "compaction_started",
        AgentMetricCount::ToolResultTruncated => "tool_result_truncated",
        _ => return None,
    })
}

fn useful_name(useful: AgentMetricUseful) -> Option<&'static str> {
    Some(match useful {
        AgentMetricUseful::NonWhitespaceText => "non_whitespace_text",
        AgentMetricUseful::SuccessfulTool => "successful_tool",
        _ => return None,
    })
}

fn attempt_name(outcome: AgentAttemptOutcome) -> Option<&'static str> {
    Some(match outcome {
        AgentAttemptOutcome::Succeeded => "succeeded",
        AgentAttemptOutcome::Failed => "failed",
        AgentAttemptOutcome::Cancelled => "cancelled",
        _ => return None,
    })
}
