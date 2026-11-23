//! The workspace summary a session gathers at launch.
//!
//! The scan itself is the chat layer's ([`crate::chat::workspace_summary`]);
//! what this module adds is the one part of it only a run can state: the Git
//! facts, read through the same Git service every other read of the repository
//! goes through, best effort and inside the scan's own time bound.
//!
//! It is started **at the run's open**, on its own thread, and it is bounded
//! three ways before it says anything: the entries it lists, the bytes it
//! spends, and the time it may take. A scan that is still running when the
//! first message is sent is simply not carried — nobody waits for it.

use std::sync::Arc;

use crate::chat::{GitFacts, SUMMARY_TIME, SummaryLimits, WorkspaceSummaryCell, scan_workspace};
use saya_agent::CancellationToken;
use saya_harness::coding::git::GitBranchState;

use super::NativeCodingJob;

impl NativeCodingJob {
    /// The run's one summary scan, started now and landed in the cell when it
    /// finishes — or `None` for a run that carries none: a surface that states
    /// its own context (a job surface), and a run with no workspace to
    /// summarise.
    pub(super) fn workspace_summary(
        self: &Arc<Self>,
        wanted: bool,
    ) -> Option<WorkspaceSummaryCell> {
        if !wanted || !self.has_workspace() {
            return None;
        }
        let job = Arc::clone(self);
        let folder = self.application.root_path().to_path_buf();
        Some(WorkspaceSummaryCell::spawn(move || {
            let git = job.git_facts();
            scan_workspace(&folder, SummaryLimits::default(), git)
        }))
    }

    /// The repository's branch and how many paths differ from the index, read
    /// through the Git service the run already holds: the application's own
    /// repository for a checkout, and the folder's repository for an in-place
    /// root that sits inside one.
    ///
    /// Best effort and bounded by [`SUMMARY_TIME`]: a folder that is not a
    /// repository, a repository the service refuses, and one that answers too
    /// slowly all state `None`, so the summary names the folder without Git
    /// rather than holding the scan open.
    fn git_facts(&self) -> Option<GitFacts> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .ok()?;
        runtime.block_on(async {
            let application = &self.application;
            let repository = match application.root.git() {
                Some(repository) => repository.clone(),
                None => {
                    let local = application.local_root()?;
                    application
                        .service
                        .open_repository(local.path(), &CancellationToken::new())
                        .await
                        .ok()?
                }
            };
            let inventory = tokio::time::timeout(
                SUMMARY_TIME,
                application
                    .service
                    .inventory_repository(&repository, &CancellationToken::new()),
            )
            .await
            .ok()?
            .ok()?;
            Some(facts(&inventory))
        })
    }
}

/// The summary's view of one inventory: the branch it is on, and every path the
/// index does not agree with — ignored paths are not changes, so they are not
/// counted.
fn facts(inventory: &saya_harness::coding::git::GitRepositoryInventory) -> GitFacts {
    let branch = match inventory.branch() {
        GitBranchState::Attached(name) | GitBranchState::Unborn(name) => Some(name.clone()),
        GitBranchState::Detached => None,
        // A branch state this release does not know states no branch rather
        // than failing the summary: the folder is named without it.
        _ => None,
    };
    let changed = inventory
        .paths()
        .iter()
        .filter(|path| !path.ignored())
        .count();
    GitFacts { branch, changed }
}
