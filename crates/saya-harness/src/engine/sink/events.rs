use async_trait::async_trait;
use saya_agent::{AgentEvent, AgentEventSink, AnsweringObservation};
use saya_types::{PauseReason, RunEvent};

use crate::engine::usage::AnsweringCountError;
use crate::engine::{state::RunState, transitions::TransitionEvent};

use super::{EngineEventSink, EngineSinkError};

#[async_trait]
impl AgentEventSink for EngineEventSink {
    async fn observe_answering(&self, observation: AnsweringObservation) {
        if !*self
            .count_tracking
            .lock()
            .expect("engine sink answering-count tracking lock")
        {
            return;
        }
        let mut counts = self
            .answering_counts
            .lock()
            .expect("engine sink answering-count lock");
        let mut updated = *counts;
        match updated.observe(observation) {
            Ok(false) => return,
            Ok(true) => {}
            Err(source) => {
                self.hold_diagnostic(EngineSinkError::AnsweringCount { source });
                self.request_owner_failure();
                return;
            }
        }
        let (turns, tool_calls) = match observation {
            AnsweringObservation::Initialized => (Some(0), Some(0)),
            AnsweringObservation::RequestAdmitted => (Some(1), None),
            AnsweringObservation::ToolBatchObserved { requested } => (None, Some(requested)),
            _ => {
                self.hold_diagnostic(EngineSinkError::AnsweringCount {
                    source: AnsweringCountError::InvalidEvidence,
                });
                self.request_owner_failure();
                return;
            }
        };
        let event = RunEvent::Usage {
            endpoint: crate::endpoints::ORCHESTRATOR_ROLE.to_string(),
            tokens: None,
            turns,
            tool_calls,
            cached_input_tokens: None,
            cache_creation_input_tokens: None,
        };
        match self.journal.append(&event) {
            Ok(()) => *counts = updated,
            Err(source) => {
                self.hold_diagnostic(EngineSinkError::Journal { source });
                self.request_owner_failure();
            }
        }
    }

    /// One event emission is one tick: a successful or failed-attempt usage
    /// receipt folds into the run's totals and is journaled as the durable
    /// per-endpoint record, the download wallet's spend is journaled when it
    /// has grown past the level the record holds, and then the wall-clock
    /// deadline is checked. Counting precedes the check because the tokens
    /// the event reports were already spent.
    async fn emit(&self, event: AgentEvent) {
        if let AgentEvent::Usage { usage, .. } | AgentEvent::FailedAttemptUsage { usage } = &event {
            self.usage
                .lock()
                .expect("engine sink usage lock")
                .fold(usage);
            if let Err(error) = self.journal_usage(usage) {
                self.hold_diagnostic(error);
                self.request_owner_failure();
                return;
            }
        }
        if let Err(error) = self.journal_downloads() {
            self.hold_diagnostic(error);
            self.request_owner_failure();
            return;
        }
        let mut clock_error = None;
        if matches!(
            &event,
            AgentEvent::TurnStarted | AgentEvent::ToolRequested { .. }
        ) && let Err(error) = self.observe_elapsed_clock(true)
        {
            clock_error = Some(error);
        }
        if let Some(error) = clock_error {
            self.hold_diagnostic(error);
            if self.cancellation.is_none()
                && self.state() == RunState::Executing
                && let Err(error) = self
                    .record(TransitionEvent::Pause(PauseReason::WallClockExceeded))
                    .await
            {
                self.hold_diagnostic(error);
            }
        }
        self.observe_stop_before_display();
        if let Some(stream) = &self.agent_stream {
            stream.emit(event).await;
        }
        self.tick().await;
    }
}
