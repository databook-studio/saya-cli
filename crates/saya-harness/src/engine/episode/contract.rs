//! The episode driver's public contract: its typed errors and the inputs
//! the composition root supplies once per run.
//!
//! These types are constructible by design — they are the composition
//! root's side of the driver, not data the model or plan produces.

use std::sync::Arc;

use saya_agent::{ApprovalDecider, CancellationToken, ChatProvider, ToolDefinition, ToolExecutor};
use saya_store::RunStore;
use saya_types::{RunFailureCode, RunId};
use thiserror::Error;

use crate::{HarnessError, journal::Journal};

use crate::engine::sink::EngineSinkError;
use crate::engine::state::RunState;

/// Why the episode driver refused or gave up on a step. Data, not prose:
/// `saya-cli` renders these.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum EpisodeError {
    /// The step failed on every bounded attempt; the run was paused with
    /// `StepFailedAfterRetry`. `code` is the typed cause of the last
    /// episode's failure.
    #[error("step {step} failed after {attempts} attempts; the run paused")]
    StepExhausted {
        step: usize,
        attempts: usize,
        code: RunFailureCode,
    },

    /// The user cancelled the run; the run was recorded cancelled.
    #[error("run cancelled during step {step}")]
    Cancelled { step: usize },

    /// The run cannot begin a step in its current state: `planned` has no
    /// approval to run under, a later step cannot begin an unstarted run,
    /// and `paused` resumes elsewhere (a later slice).
    #[error("run cannot start step {step} while {state}")]
    NotRunnable { step: usize, state: RunState },

    /// The step index is beyond the plan.
    #[error("step {step} is beyond the plan's {steps} steps")]
    OutOfRange { step: usize, steps: usize },

    /// The brief could not be built: the workspace walk refused.
    #[error("episode brief failed: {source}")]
    Brief { source: HarnessError },

    /// A journal write failed; the durable record did not advance.
    #[error("run journal write failed: {source}")]
    Journal { source: HarnessError },

    /// The state store refused a step mirror.
    #[error("run state store refused a step mirror: {source}")]
    Store { source: saya_store::StoreError },

    /// A lifecycle transition was refused — by the machine, its journal
    /// write, or its store mirror.
    #[error("run transition failed: {source}")]
    Transition { source: EngineSinkError },
}

/// The loop's collaborators and the run's tool universe, supplied once per
/// run by the composition root and shared by every episode.
pub struct EpisodeCollaborators<'a> {
    pub provider: &'a dyn ChatProvider,
    pub tools: &'a dyn ToolExecutor,
    pub approval: &'a dyn ApprovalDecider,
    /// Every tool definition the run may advertise. `run_step` narrows it
    /// to the step's capabilities: a tool outside them is absent from the
    /// episode's definitions, never present-and-refused.
    pub universe: Vec<ToolDefinition>,
    pub cancellation: CancellationToken,
}

/// The run's identity and its two mirrors. The store is the same
/// `Arc<dyn RunStore>` the sink mirrors lifecycle status through; the
/// driver mirrors the step rows over it.
pub struct EpisodeRun {
    pub run_id: RunId,
    pub store: Arc<dyn RunStore>,
    pub journal: Journal,
}

/// The per-run request inputs the engine does not derive from the step:
/// the model (per-step endpoint→provider/model selection lands with the
/// `PromptOverrides` seam) and the profiles the episode's tools target.
/// `memory_allows_candidate_writes` is the caller's memory-mode-derived
/// permission, carried so callers need not special-case runs — an episode
/// pins learning off by construction (DESIGN §5.8) and never consults it.
pub struct EpisodeRequest {
    pub model: String,
    pub profile_names: Vec<String>,
    pub memory_allows_candidate_writes: bool,
}

/// How much workspace the brief's manifest will walk: the file count and
/// the per-file byte bound. The caller supplies them; the driver invents
/// none.
pub struct ManifestBounds {
    pub max_files: usize,
    pub max_file_bytes: u64,
}
