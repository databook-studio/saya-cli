//! The episode driver: one plan step, driven end to end through the agent
//! loop.
//!
//! Per step the driver builds the brief (the plan state plus the workspace
//! manifest), runs the step's own toolset — the executor and definitions the
//! composition root built from the step's capabilities, so a tool outside
//! them is hidden, not advertised-and-refused — turns the step's budget into
//! `AgentLimits` with no environment input (a run is reproducible from its
//! spec and config alone, plan G3), pins learning off explicitly (DESIGN
//! §5.8), and retries a failing episode a bounded number of times with a
//! fresh brief. When the bound is spent the run pauses with a typed failure
//! code; it never retries unbounded and never fails silently.
//!
//! The lifecycle transitions the driver records through the sink: `Begin`
//! (an approved run starts executing with its first episode), `Complete`
//! (the plan's last step succeeded), `Pause` (`StepFailedAfterRetry`), and
//! `Cancel` (the user stopped the episode). Mid-run steps need no run-level
//! transition — the steps' `StepStarted` journal events tell that story.

mod attempt_loop;
mod brief;
mod contract;
mod deliverables;
mod drive;
mod retry;
mod stop;

pub use contract::{
    EpisodeCollaborators, EpisodeError, EpisodeRequest, EpisodeRun, ManifestBounds, StepToolset,
};
pub use retry::MAX_EPISODE_ATTEMPTS;

/// Drives one plan step: the brief, the narrowed tools, the step budget,
/// and the bounded fresh-brief retries, through the engine's event sink.
pub struct EpisodeDriver<'a> {
    collaborators: EpisodeCollaborators<'a>,
    run: EpisodeRun,
    request: EpisodeRequest,
    bounds: contract::ManifestBounds,
}

impl<'a> EpisodeDriver<'a> {
    /// Assembles the driver from the pieces the composition root owns.
    pub fn new(
        collaborators: EpisodeCollaborators<'a>,
        run: EpisodeRun,
        request: EpisodeRequest,
        bounds: contract::ManifestBounds,
    ) -> Self {
        Self {
            collaborators,
            run,
            request,
            bounds,
        }
    }
}
