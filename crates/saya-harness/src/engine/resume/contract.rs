//! The resume entry point's public contract: its typed outcome and error,
//! and the inputs the composition root supplies once per resume.
//!
//! These types are constructible by design — the composition root's side of
//! the resume, not data the model or plan produces.

use std::{sync::Arc, time::Duration};

use saya_store::RunStore;
use saya_types::{RunId, RunPlan};

use crate::{HarnessError, workspace::Workspace};

use crate::engine::episode::{EpisodeCollaborators, EpisodeError, EpisodeRequest, ManifestBounds};
use crate::engine::sink::EngineSinkError;
use crate::engine::state::RunState;

/// What resume found in the journal and what it did about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeOutcome {
    /// The run continues at the plan's first incomplete step — an in-flight
    /// step restarted from its beginning — and every remaining step ran
    /// through the episode driver. `state` is where the run ended.
    Resumed { first_step: usize, state: RunState },
    /// Nothing was left to run: the journal already records a terminal
    /// state, or every step is complete and resume recorded the completion
    /// the crash had left unwritten.
    Settled { state: RunState },
    /// The journal's run has no approval on record. Approval is explicit;
    /// resume runs nothing and records nothing.
    Unapproved,
    /// The journal holds no run.
    NoRun,
}

/// Why resume refused or stopped. Data, not prose: `saya-cli` renders these.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ResumeError {
    /// The run directory's single-writer lock is held by a live process:
    /// another engine owns the run.
    #[error("run directory is locked by another engine: {source}")]
    Lock {
        #[source]
        source: HarnessError,
    },
    /// The journal could not be read, repaired, or replayed.
    #[error("run journal replay failed: {source}")]
    Journal {
        #[source]
        source: HarnessError,
    },
    /// A resumed step failed, or a gate refused it.
    #[error("resumed episode failed: {source}")]
    Episode {
        #[source]
        source: EpisodeError,
    },
    /// A lifecycle transition was refused — by the machine, its journal
    /// write, or its store mirror.
    #[error("run transition failed: {source}")]
    Transition {
        #[source]
        source: EngineSinkError,
    },
}

/// The composition root's side of a resume: everything but the journal and
/// the machine state, which resume derives from the journal itself.
pub struct ResumeRun<'a> {
    pub run_id: RunId,
    pub store: Arc<dyn RunStore>,
    pub plan: RunPlan,
    pub workspace: Workspace,
    pub collaborators: EpisodeCollaborators<'a>,
    pub request: EpisodeRequest,
    pub bounds: ManifestBounds,
    /// The run's declared wall-clock ceiling, armed again for the resumed steps.
    pub wall_clock: Option<Duration>,
}
