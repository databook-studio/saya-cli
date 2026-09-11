//! The run engine.
//!
//! The state machine is a pure transition table over the run lifecycle: no
//! I/O, no async. The event sink is the engine's `AgentEventSink` — it
//! counts usage across the run, enforces the wall-clock deadline per tick,
//! and records lifecycle transitions into the journal and the store. The
//! episode driver, resume and plan validation land on top of this.

mod episode;
mod plan;
mod resume;
mod sink;
mod state;
mod transitions;
mod usage;

pub use episode::{
    EpisodeCollaborators, EpisodeDriver, EpisodeError, EpisodeRequest, EpisodeRun,
    MAX_EPISODE_ATTEMPTS, ManifestBounds, StepToolset,
};
pub use plan::{
    MAX_PLAN_ATTEMPTS, PlanDriver, PlanError, PlanParseFailure, PlanRejection, PlanRequest,
};
pub use resume::{ResumeError, ResumeOutcome, ResumeRun, resume};
pub use sink::{EngineEventSink, EngineSinkError};
pub use state::{RunState, RunTransition, RunTransitionError, transition};
pub use transitions::TransitionEvent;
pub use usage::UsageTotals;
