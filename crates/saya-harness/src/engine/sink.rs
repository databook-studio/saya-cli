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
    /// The run's token ceiling, summed across input and output.
    ///
    /// The declared budget is per endpoint, but every episode currently calls
    /// the single orchestrator endpoint, so there is exactly one bucket to
    /// enforce and the ceiling is that endpoint's. When per-step endpoint
    /// roles bind, this becomes a map and attribution follows the call —
    /// until then a per-endpoint ceiling with one endpoint is the same
    /// number, and pretending otherwise would be the more confusing lie.
    token_ceiling: Option<u64>,
    state: Mutex<RunState>,
    usage: Mutex<UsageTotals>,
    diagnostic: Mutex<Option<Arc<EngineSinkError>>>,
    /// An optional downstream sink every agent event is forwarded to after
    /// usage folds in. The headless run wire (`saya-cli`) attaches one that
    /// renders the episode's events in the caller's format; the sink's own
    /// accounting (usage totals, the wall-clock tick) is unchanged either way.
    agent_stream: Option<Arc<dyn AgentEventSink>>,
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
        token_ceiling: Option<u64>,
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
            token_ceiling,
            state: Mutex::new(initial),
            usage: Mutex::new(UsageTotals::default()),
            diagnostic: Mutex::new(None),
            agent_stream: None,
        }
    }

    /// Attaches the downstream sink every agent event forwards to. Usage
    /// accounting and the wall-clock tick are unaffected; the observer only
    /// ever mirrors.
    pub fn with_agent_stream(mut self, stream: Arc<dyn AgentEventSink>) -> Self {
        self.agent_stream = Some(stream);
        self
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
        if let Some(stream) = &self.agent_stream {
            stream.emit(event).await;
        }
        self.tick().await;
    }
}

impl EngineEventSink {
    /// The per-tick budget checks, armed while the run executes. Past either
    /// ceiling the tick pauses the run exactly like any other transition;
    /// emit has no error channel, so any failure the pause meets is held as
    /// a diagnostic rather than dropped.
    ///
    /// Tokens are checked after the fold, never before: the tokens an event
    /// reports were already spent, so the ceiling is a stop-after, not a
    /// stop-before. A run pauses the tick *after* it crosses — which is the
    /// honest reading of a ceiling nobody can enforce mid-request.
    async fn tick(&self) {
        if self.state() != RunState::Executing {
            return;
        }
        let reason = if self.tokens_exhausted() {
            PauseReason::BudgetExhausted
        } else if self
            .deadline
            .is_some_and(|deadline| (self.clock)() >= deadline)
        {
            PauseReason::WallClockExceeded
        } else {
            return;
        };
        if let Err(error) = self.record(TransitionEvent::Pause(reason)).await {
            self.hold_diagnostic(error);
        }
    }

    /// Whether the run has spent its declared token ceiling. Input and output
    /// are summed because the budget is what the run costs, and a ceiling
    /// that counted only one half would be a ceiling on nothing in
    /// particular. Figures no call reported stay out of the sum rather than
    /// counting as zero.
    fn tokens_exhausted(&self) -> bool {
        let Some(ceiling) = self.token_ceiling else {
            return false;
        };
        let usage = self.usage();
        usage.input_tokens.saturating_add(usage.output_tokens) >= ceiling
    }
}
