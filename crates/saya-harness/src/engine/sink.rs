//! The engine's event sink: where the agent loop's event stream meets the
//! run.
//!
//! The episode driver hands this sink to `run_agent_with_sink`; every event
//! the loop emits passes through [`EngineEventSink::emit`], and each
//! emission is a tick. The sink counts usage across the run, enforces the
//! run's wall-clock deadline per tick, and records lifecycle transitions:
//! each one advances the state machine, is appended to the run journal, and
//! is mirrored to the state store.
//!
//! Failure posture: when the store refuses a mirror while the run is
//! mid-flight, the run **pauses** with a diagnostic — the journal records
//! `Paused { reason: StoreUnavailable }` and the state advances to `paused`.
//! A gate is never weakened to keep going, and nothing proceeds silently.
//! The journal is the durable authority; the store is the mirror a resume
//! reconciles. The deadline trip is the same pause with
//! `PauseReason::WallClockExceeded`, and a paused run never re-pauses.
//!
//! The journal event for a transition belongs to the sink (the state
//! machine's doc): `Approve`, `Pause`, `Complete`, `Fail`, and `Cancel` each
//! append theirs. `Begin` and `Resume` carry none — there is no journal
//! event for bare `Executing`; the steps' `StepStarted` events tell that
//! story — so they mirror `Executing` to the store only.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use saya_agent::{AgentEvent, AgentEventSink};
use saya_store::{RunStore, StoreError};
use saya_types::{PauseReason, RunEvent, RunId};

use crate::{HarnessError, journal::Journal};

use super::state::{RunState, RunTransitionError, transition};
use super::transitions::TransitionEvent;
use super::usage::UsageTotals;

/// The wall clock the sink reads. Production passes `Instant::now`; tests
/// inject a simulated clock so a deadline trips without sleeping.
type Clock = Box<dyn Fn() -> Instant + Send + Sync>;

/// Errors from recording a transition. Data, not prose: `saya-cli` renders
/// them.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EngineSinkError {
    /// The state machine refused the transition.
    #[error("run transition refused: {source}")]
    Transition {
        #[source]
        source: RunTransitionError,
    },

    /// The journal write failed; the transition was not durably recorded,
    /// so the state did not advance.
    #[error("run journal write failed: {source}")]
    Journal {
        #[source]
        source: HarnessError,
    },

    /// The store refused a write. The run paused rather than proceeding
    /// (the module docs' failure posture); the sink also holds this as its
    /// diagnostic.
    #[error("run state store refused a write; the run paused rather than proceeding: {source}")]
    Store {
        #[source]
        source: StoreError,
    },
}

/// The engine's `AgentEventSink`. Shared as `Arc<EngineEventSink>` by whoever
/// holds both the trait object and the engine-facing surface; all state is
/// interior.
pub struct EngineEventSink {
    run_id: RunId,
    journal: Journal,
    store: Arc<dyn RunStore>,
    clock: Clock,
    deadline: Option<Instant>,
    state: Mutex<RunState>,
    usage: Mutex<UsageTotals>,
    diagnostic: Mutex<Option<Arc<EngineSinkError>>>,
}

impl EngineEventSink {
    /// A sink for `run_id` standing at `initial` — the state the machine
    /// already holds. `journal` and `store` are the run's two mirrors;
    /// `wall_clock` is the run's declared ceiling, measured against `clock`.
    /// A ceiling so large it cannot be added to the current instant arms as
    /// already expired: a budget that cannot be honored trips rather than
    /// runs unbounded.
    pub fn new(
        run_id: RunId,
        initial: RunState,
        journal: Journal,
        store: Arc<dyn RunStore>,
        wall_clock: Option<Duration>,
        clock: impl Fn() -> Instant + Send + Sync + 'static,
    ) -> Self {
        let now = clock();
        let deadline = wall_clock.map(|ceiling| now.checked_add(ceiling).unwrap_or(now));
        Self {
            run_id,
            journal,
            store,
            clock: Box::new(clock),
            deadline,
            state: Mutex::new(initial),
            usage: Mutex::new(UsageTotals::default()),
            diagnostic: Mutex::new(None),
        }
    }

    /// The run's state as the sink last advanced it.
    pub fn state(&self) -> RunState {
        *self.state.lock().expect("engine sink state lock")
    }

    /// Usage counted across the run's provider calls so far.
    pub fn usage(&self) -> UsageTotals {
        *self.usage.lock().expect("engine sink usage lock")
    }

    /// Takes the last failure the sink held — one it had no error channel to
    /// return through, or a store failure `record` surfaced and also held.
    /// `None` when nothing failed.
    pub fn take_diagnostic(&self) -> Option<Arc<EngineSinkError>> {
        self.diagnostic
            .lock()
            .expect("engine sink diagnostic lock")
            .take()
    }

    fn hold_diagnostic(&self, error: EngineSinkError) {
        *self.diagnostic.lock().expect("engine sink diagnostic lock") = Some(Arc::new(error));
    }

    fn set_state(&self, state: RunState) {
        *self.state.lock().expect("engine sink state lock") = state;
    }

    /// Records one lifecycle transition: the machine advances, the journal
    /// event is appended, the store is mirrored — the durable record always
    /// precedes the mirror, so a refused transition, a failed journal write,
    /// or a refused mirror returns an error and leaves the state as it was.
    ///
    /// The exception is the store fail-safe: a mirror the store refuses
    /// while the run is mid-flight (now `Executing`) pauses the run instead
    /// of continuing unmirrored — journaled first, then the state advances.
    pub async fn record(&self, event: TransitionEvent) -> Result<RunState, EngineSinkError> {
        let (machine, journal_event, status, code) = event.record();
        let current = self.state();
        let next = transition(current, machine)
            .map_err(|source| EngineSinkError::Transition { source })?;
        if let Some(event) = journal_event {
            self.journal
                .append(&event)
                .map_err(|source| EngineSinkError::Journal { source })?;
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

#[async_trait]
impl AgentEventSink for EngineEventSink {
    /// One event emission is one tick: usage folds into the run's totals,
    /// then the wall-clock deadline is checked. Counting precedes the check
    /// because the tokens the event reports were already spent.
    async fn emit(&self, event: AgentEvent) {
        if let AgentEvent::Usage { usage, .. } = &event {
            self.usage
                .lock()
                .expect("engine sink usage lock")
                .fold(usage);
        }
        self.tick().await;
    }
}

impl EngineEventSink {
    /// The per-tick wall-clock check, armed while the run executes. Past the
    /// deadline the tick pauses the run exactly like any other transition;
    /// emit has no error channel, so any failure the pause meets is held as
    /// a diagnostic rather than dropped.
    async fn tick(&self) {
        let Some(deadline) = self.deadline else {
            return;
        };
        if (self.clock)() < deadline || self.state() != RunState::Executing {
            return;
        }
        if let Err(error) = self
            .record(TransitionEvent::Pause(PauseReason::WallClockExceeded))
            .await
        {
            self.hold_diagnostic(error);
        }
    }
}
