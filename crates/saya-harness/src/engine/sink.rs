//! The engine's event sink: where the agent loop's event stream meets the
//! run.
//!
//! The episode driver hands this sink to `run_agent_with_sink`; every event
//! the loop emits passes through [`EngineEventSink::emit`], and each
//! emission is a tick. The sink counts usage across the run, journals the
//! download wallet's spend as it grows, enforces budgets per tick, and
//! signals the attached episode owner when a controlled run must stop. A
//! legacy unattached sink still pauses on its own tick. The sink records
//! lifecycle transitions: each one advances
//! the state machine, is appended to the run journal, and is mirrored to
//! the state store.
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
    time::Instant,
};

use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AnsweringObservation, BudgetDimension, CancellationReason,
    CancellationToken,
};
use saya_store::RunStore;
use saya_types::{PauseReason, RunEvent, RunId};

use crate::fetch::DownloadBudget;
use crate::journal::Journal;

use super::clock::ElapsedClock;
use super::usage::{AnsweringCountError, StepAnsweringCounts, UsageTotals};
use super::{state::RunState, transitions::TransitionEvent};

mod accounting;
mod enforcement;
mod lifecycle;
mod types;

/// The wall clock the sink reads. Production passes `Instant::now`; tests
/// inject a simulated clock so a deadline trips without sleeping.
type Clock = Box<dyn Fn() -> Instant + Send + Sync>;

pub use types::{EngineSinkError, SinkBudgets};

/// The engine's `AgentEventSink`. Shared as `Arc<EngineEventSink>` by whoever
/// holds both the trait object and the engine-facing surface; all state is
/// interior.
pub struct EngineEventSink {
    run_id: RunId,
    journal: Journal,
    store: Arc<dyn RunStore>,
    clock: Clock,
    deadline: Option<Instant>,
    /// The run's token ceiling, armed from [`SinkBudgets`] — its per-endpoint
    /// and whole-spend semantics are documented there.
    token_ceiling: Option<u64>,
    /// The run's download wallet, armed from [`SinkBudgets`]; `None` is the
    /// inert case (the run did not approve fetch).
    download_budget: Option<DownloadBudget>,
    state: Mutex<RunState>,
    usage: Mutex<UsageTotals>,
    answering_counts: Mutex<StepAnsweringCounts>,
    count_tracking: Mutex<bool>,
    /// The download level the journal already holds when this sink takes
    /// over — the wallet's consumed figure at construction, which on a
    /// resume is the level its record carries (the seeding `resume` did
    /// before this sink was built) and on a fresh run is zero. Each tick
    /// journals the wallet's growth past this level, so the sink records
    /// only the spend it observed and never re-journals the level the
    /// record already holds.
    journaled_downloads: Mutex<u64>,
    diagnostic: Mutex<Option<Arc<EngineSinkError>>>,
    /// An optional downstream sink every agent event is forwarded to after
    /// usage folds in. The headless run wire (`saya-cli`) attaches one that
    /// renders the episode's events in the caller's format; the sink's own
    /// accounting (usage totals, the wall-clock tick) is unchanged either way.
    agent_stream: Option<Arc<dyn AgentEventSink>>,
    elapsed_clock: Mutex<Option<ElapsedClock>>,
    cancellation: Option<CancellationToken>,
}

impl EngineEventSink {
    /// A sink for `run_id` standing at `initial` — the state the machine
    /// already holds. `journal` and `store` are the run's two mirrors;
    /// `budgets` carries the run's declared ceilings and the spend already
    /// behind them — the seeding a resume does, so the token ceiling
    /// measures the run's whole spend; a fresh run carries
    /// [`UsageTotals::default`]. For direct/legacy callers, `wall_clock`
    /// remains an invocation-local monotonic deadline. The product CLI and
    /// engine resume path attach an [`ElapsedClock`] for journal-owned
    /// whole-run elapsed carry with [`Self::with_elapsed_clock`].
    pub fn new(
        run_id: RunId,
        initial: RunState,
        journal: Journal,
        store: Arc<dyn RunStore>,
        budgets: SinkBudgets,
        clock: impl Fn() -> Instant + Send + Sync + 'static,
    ) -> Self {
        let SinkBudgets {
            wall_clock,
            token_ceiling,
            download_budget,
            carried_usage,
        } = budgets;
        let now = clock();
        let deadline = wall_clock.map(|ceiling| now.checked_add(ceiling).unwrap_or(now));
        let journaled_downloads = Mutex::new(
            download_budget
                .as_ref()
                .map_or(0, |budget| budget.consumed()),
        );
        Self {
            run_id,
            journal,
            store,
            clock: Box::new(clock),
            deadline,
            token_ceiling,
            download_budget,
            state: Mutex::new(initial),
            usage: Mutex::new(carried_usage),
            answering_counts: Mutex::new(StepAnsweringCounts::default()),
            count_tracking: Mutex::new(false),
            journaled_downloads,
            diagnostic: Mutex::new(None),
            agent_stream: None,
            elapsed_clock: Mutex::new(None),
            cancellation: None,
        }
    }

    /// Attaches the durable whole-run clock used by configured runs.
    pub fn with_elapsed_clock(self, clock: ElapsedClock) -> Self {
        *self.elapsed_clock.lock().expect("engine sink clock lock") = Some(clock);
        self
    }

    pub(crate) fn with_answering_counts(self, counts: StepAnsweringCounts) -> Self {
        *self
            .answering_counts
            .lock()
            .expect("engine sink answering-count lock") = counts;
        self
    }

    pub(in crate::engine) fn begin_answering_step(&self, step: usize, tracked: bool) {
        self.answering_counts
            .lock()
            .expect("engine sink answering-count lock")
            .begin_step(step);
        *self
            .count_tracking
            .lock()
            .expect("engine sink answering-count tracking lock") = tracked;
    }

    pub(in crate::engine) fn remaining_answering_grant(
        &self,
        step: usize,
        turns: Option<u64>,
        tool_calls: Option<u64>,
    ) -> Result<(Option<u64>, Option<u64>), AnsweringCountError> {
        self.answering_counts
            .lock()
            .expect("engine sink answering-count lock")
            .remaining(step, turns, tool_calls)
    }

    pub(crate) async fn settle_failed_request(&self) {
        let tracked = *self.count_tracking.lock().expect("tracking lock");
        let pending = self
            .answering_counts
            .lock()
            .expect("counts lock")
            .is_pending();
        if tracked && pending {
            self.observe_answering(AnsweringObservation::ToolBatchObserved { requested: 0 })
                .await;
        }
    }

    /// Attaches the production run's shared cancellation owner. Legacy sink
    /// callers without this attachment keep the historical per-tick pause.
    pub fn with_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    pub(super) fn request_budget_stop(&self, dimension: BudgetDimension) -> bool {
        self.cancellation.as_ref().is_some_and(|cancellation| {
            cancellation.cancel_with_reason(CancellationReason::Budget(dimension));
            true
        })
    }

    pub(in crate::engine) fn has_cancellation(&self) -> bool {
        self.cancellation.is_some()
    }

    pub(in crate::engine) fn request_owner_failure(&self) {
        if let Some(cancellation) = &self.cancellation {
            cancellation.cancel_with_reason(CancellationReason::OwnerFailure);
        }
    }

    pub(in crate::engine) fn cancellation_reason(&self) -> Option<CancellationReason> {
        self.cancellation
            .as_ref()
            .and_then(CancellationToken::reason)
    }

    /// Attaches the downstream sink every agent event forwards to.
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

    pub(in crate::engine) fn hold_diagnostic(&self, error: EngineSinkError) {
        self.hold_diagnostic_arc(Arc::new(error));
    }

    pub(in crate::engine) fn hold_diagnostic_arc(&self, error: Arc<EngineSinkError>) {
        let mut diagnostic = self.diagnostic.lock().expect("engine sink diagnostic lock");
        if diagnostic.is_none() {
            *diagnostic = Some(error);
        }
    }

    fn set_state(&self, state: RunState) {
        *self.state.lock().expect("engine sink state lock") = state;
    }
}

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

impl EngineEventSink {
    fn observe_stop_before_display(&self) {
        if self.state() != RunState::Executing {
            return;
        }
        let already_stopped = self.cancellation_reason().is_some();
        let dimension = if already_stopped {
            None
        } else if self.tokens_exhausted() {
            Some(BudgetDimension::ReportedTokens)
        } else if self.downloads_exhausted() {
            Some(BudgetDimension::DownloadBytes)
        } else {
            None
        };
        if let Some(dimension) = dimension {
            self.request_budget_stop(dimension);
        }
        match self.observe_elapsed_clock(false) {
            Ok(true) => {
                if !already_stopped && dimension.is_none() {
                    self.request_budget_stop(BudgetDimension::ElapsedTime);
                }
            }
            Ok(false) => {
                if self
                    .deadline
                    .is_some_and(|deadline| (self.clock)() >= deadline)
                    && !already_stopped
                    && dimension.is_none()
                {
                    self.request_budget_stop(BudgetDimension::ElapsedTime);
                }
            }
            Err(error) => {
                self.hold_diagnostic(error);
                if !already_stopped && dimension.is_none() {
                    self.request_owner_failure();
                }
            }
        }
    }
}
