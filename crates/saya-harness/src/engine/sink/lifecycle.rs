use saya_types::{PauseReason, RunEvent};

use super::{EngineEventSink, EngineSinkError};
use crate::engine::{
    state::{RunState, transition},
    transitions::TransitionEvent,
};

impl EngineEventSink {
    /// Records a lifecycle transition journal-first, then mirrors it to the
    /// store. Meaningful boundaries force any raised clock observation first.
    pub async fn record(&self, event: TransitionEvent) -> Result<RunState, EngineSinkError> {
        let force_clock_observation = matches!(
            &event,
            TransitionEvent::Begin
                | TransitionEvent::Resume
                | TransitionEvent::Complete
                | TransitionEvent::Cancel
                | TransitionEvent::Pause(_)
                | TransitionEvent::Fail(_)
        );
        let is_pause = matches!(&event, TransitionEvent::Pause(_));
        let (machine, journal_event, status, code) = event.record();
        let next = transition(self.state(), machine)
            .map_err(|source| EngineSinkError::Transition { source })?;
        let approved = matches!(&journal_event, Some(RunEvent::PlanApproved { .. }));
        if force_clock_observation && let Err(error) = self.observe_elapsed_clock(true) {
            if is_pause {
                self.hold_diagnostic(error);
            } else {
                return Err(error);
            }
        }
        if let Some(event) = journal_event {
            self.journal
                .append(&event)
                .map_err(|source| EngineSinkError::Journal { source })?;
        }
        if approved {
            let mut clock = self.elapsed_clock.lock().expect("engine sink clock lock");
            if let Some(clock) = clock.as_mut() {
                self.journal
                    .append(&clock.event())
                    .map_err(|source| EngineSinkError::Journal { source })?;
                clock.mark_persisted((self.clock)());
            }
        }
        if let Err(source) = self.store.set_run_status(&self.run_id, status, code).await {
            self.hold_diagnostic(EngineSinkError::Store {
                source: source.clone(),
            });
            if next == RunState::Executing {
                self.journal
                    .append(&RunEvent::Paused {
                        reason: PauseReason::StoreUnavailable,
                    })
                    .map_err(|source| EngineSinkError::Journal { source })?;
                self.set_state(RunState::Paused);
            } else {
                self.set_state(next);
            }
            return Err(EngineSinkError::Store { source });
        }
        self.set_state(next);
        Ok(next)
    }
}
