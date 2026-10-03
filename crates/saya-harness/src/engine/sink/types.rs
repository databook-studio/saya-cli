use std::time::Duration;

use crate::engine::clock::ElapsedClockError;
use crate::engine::state::RunTransitionError;
use saya_store::StoreError;

use crate::engine::usage::UsageTotals;
use crate::{HarnessError, fetch::DownloadBudget};

/// Errors from recording a transition. Data, not prose: `saya-cli` renders
/// them.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EngineSinkError {
    /// The first-party elapsed clock could not be safely observed.
    #[error("run elapsed clock failed closed: {source}")]
    ElapsedClock {
        #[source]
        source: ElapsedClockError,
    },

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

/// The run's budgets as the sink arms them: the declared ceilings, and the
/// spend the run's durable record already holds. Declared together because
/// they arm together — the constructor carries no default for any of them,
/// so a caller cannot arm a ceiling without stating what is already spent.
#[derive(Debug, Clone)]
pub struct SinkBudgets {
    /// The invocation's wall-clock ceiling, measured against the sink's
    /// monotonic clock. A ceiling so large it cannot be added to the current
    /// instant arms as already expired. Whole-run carry requires attaching an
    /// [`ElapsedClock`](crate::engine::ElapsedClock) to the sink.
    pub wall_clock: Option<Duration>,
    /// The run's declared token ceiling, summed across input and output.
    ///
    /// The declared budget is per endpoint, but every episode currently
    /// calls the single orchestrator endpoint, so there is exactly one
    /// bucket to enforce and the ceiling is that endpoint's. When per-step
    /// endpoint roles bind, this becomes a map and attribution follows the
    /// call — until then a per-endpoint ceiling with one endpoint is the
    /// same number, and pretending otherwise would be the more confusing
    /// lie.
    pub token_ceiling: Option<u64>,
    /// The run's download wallet — the *same* wallet clone the fetch-capable
    /// step executors hold, so a download refusal the tool reported to the
    /// model also reaches the run's lifecycle here. `None` when the run did
    /// not approve fetch: the check is inert, the `token_ceiling: None`
    /// pattern. The check reads the wallet's **trip latch**, never a
    /// `consumed >= limit` threshold — the latch records the event that
    /// actually happened (a claim was refused), where a threshold would both
    /// pause an exact-fill run that was refused nothing and miss a refusal
    /// that left unclaimable headroom below the limit. A resume hands the
    /// wallet already carrying the spend its journal records — seeded by
    /// `resume` before this sink takes over — so the check binds the run's
    /// whole download spend, not this invocation's.
    pub download_budget: Option<DownloadBudget>,
    /// The spend the run's journal already records when this sink takes
    /// over — the seeding a resume does, so the token ceiling measures the
    /// run's whole spend across invocations rather than re-arming in full.
    /// A fresh run carries [`UsageTotals::default`].
    pub carried_usage: UsageTotals,
}
