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

mod accounting;
mod answering;
mod control;
mod diagnostic;
mod enforcement;
mod events;
mod lifecycle;
mod stop;
mod types;

/// The wall clock the sink reads. Production passes `Instant::now`; tests
/// inject a simulated clock so a deadline trips without sleeping.
type Clock = Box<dyn Fn() -> Instant + Send + Sync>;

pub use types::{EngineSinkError, SinkBudgets};

use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

use saya_agent::{AgentEventSink, CancellationToken};
use saya_store::RunStore;
use saya_types::RunId;

use crate::fetch::DownloadBudget;
use crate::journal::Journal;

use super::clock::ElapsedClock;
use super::state::RunState;
use super::usage::{StepAnsweringCounts, UsageTotals};

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

    /// Attaches the production run's shared cancellation owner. Legacy sink
    /// callers without this attachment keep the historical per-tick pause.
    pub fn with_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = Some(cancellation);
        self
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

    fn set_state(&self, state: RunState) {
        *self.state.lock().expect("engine sink state lock") = state;
    }
}
