//! The host half of a session over a folder that cannot root an in-place job:
//! the constructor that opens over such a folder at all, and the one cell every
//! workspace tool's advertisement reads.
//!
//! A folder that cannot root an in-place job — the filesystem root, the home
//! directory, saya's own state directory, a folder that would hold the job's
//! private state root, each refused by
//! [`resolve_in_place`](super::host_root::resolve_in_place) — is still a place a
//! run can answer from. A session over one composes no workspace at all: the run
//! is offered the database tools it resolved, and no file, shell or write tool.
//! What the folder's shape decides (`WorkspaceShape::workspace`), this file
//! states on the host: the folder is taken as the run's own identity, nothing is
//! captured or inventoried for it, and the cell says the run has no workspace.

use super::CodingApplication;
use crate::cli::RootMode;
use crate::config::runtime::RuntimeConfig;
use saya_agent::CancellationToken;
use saya_store::SqliteStateStore;
use std::{
    path::Path,
    sync::{Arc, atomic::Ordering},
};

impl CodingApplication {
    /// Opens the application over a folder that cannot root an in-place job.
    ///
    /// The folder is the run's label: its own canonical identity, with no
    /// baseline walked, no checkout opened, and the dangerous-root refusal not
    /// asked — nothing here is a job root. The host states `workspace: false`,
    /// so the run composed over it advertises no workspace tool.
    pub(crate) async fn session_without_workspace(
        runtime: RuntimeConfig,
        store: SqliteStateStore,
        folder: &Path,
        mode: RootMode,
        state_root: &Path,
        token: &CancellationToken,
    ) -> Result<Arc<Self>, super::super::CodingError> {
        let (root, program_dir) =
            super::super::root_mode::session_inputs_over(&runtime, state_root)?;
        Self::open_default_with_mode(
            runtime,
            store,
            folder,
            &root,
            &super::super::cli::find_git()?,
            &program_dir,
            mode,
            token,
        )
        .await
    }

    /// Whether this host's runs have a workspace at all: what every file,
    /// search, shell and write tool's advertisement reads before it offers
    /// itself.
    pub(crate) fn has_workspace(&self) -> bool {
        self.workspace.load(Ordering::Acquire)
    }

    /// States that this host's runs are one-turn runs: one question, one turn,
    /// one answer, and no outcome view recorded afterwards.
    pub(crate) fn with_one_turn(self: &Arc<Self>) {
        self.one_turn.store(true, Ordering::Release);
    }

    /// Whether this host composes one-turn runs.
    pub(crate) fn is_one_turn(&self) -> bool {
        self.one_turn.load(Ordering::Acquire)
    }
}

impl super::super::NativeCodingJob {
    /// Whether this run has a workspace at all. A session opened over a folder
    /// that cannot root an in-place job states none, and its run composes no
    /// workspace tool.
    pub(crate) fn has_workspace(&self) -> bool {
        self.application.has_workspace()
    }

    /// Whether this run is one turn of a one-turn composition.
    pub(crate) fn is_one_turn(&self) -> bool {
        self.application.is_one_turn()
    }
}
