//! The read-only guard in front of every apply-back: anchored, no-follow
//! proof that every touched path still matches baseline and capture-time git facts.

use super::apply::AcceptedDelta;
use super::delta_types::DEFAULT_DELTA_DEADLINE;
use super::index::{PlanError, PlanIndex};
use crate::coding::git::{GitInventoryError, GitRepository, GitService};
use crate::coding::snapshot::CapturedSnapshot;
use saya_agent::CancellationToken;
use std::fmt;

#[cfg(unix)]
use super::MAX_ASSIGNED_PATHS;
#[cfg(unix)]
use super::preflight_fs::{ReadState, classify_worktree};
#[cfg(unix)]
use crate::coding::git::{GitPathStatus, SnapshotIndexInput};
#[cfg(unix)]
use crate::coding::snapshot::{SnapshotFileMode, SnapshotSelection};
#[cfg(unix)]
use crate::workspace::fd::open_root_fd;
#[cfg(unix)]
use saya_types::Sha256Digest;
#[cfg(unix)]
use std::collections::{BTreeSet, HashMap};
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::time::Instant;

pub struct PreflightInput<'a> {
    pub repository: &'a GitRepository,
    pub service: &'a GitService,
    pub baseline: &'a CapturedSnapshot,
    pub plan: &'a PlanIndex,
    pub accepted: &'a [AcceptedDelta<'a>],
}

pub struct PreflightBounds {
    pub deadline: std::time::Duration,
}

impl Default for PreflightBounds {
    fn default() -> Self {
        Self {
            deadline: DEFAULT_DELTA_DEADLINE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollisionKind {
    UserEdited,
    UserDeleted,
    UserAddedUntracked,
    IndexStagedDifferent,
    TypeChanged,
}

pub struct Collision {
    path: String,
    kind: CollisionKind,
}

impl Collision {
    pub fn kind(&self) -> CollisionKind {
        self.kind
    }
    pub fn path(&self) -> &str {
        &self.path
    }
}

impl fmt::Debug for Collision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Collision")
            .field("kind", &self.kind)
            .finish()
    }
}

pub struct ApplyBackPreflight {
    checked: usize,
    collisions: Vec<Collision>,
    overflow: usize,
}

impl ApplyBackPreflight {
    pub fn is_clean(&self) -> bool {
        self.collisions.is_empty() && self.overflow == 0
    }
    pub fn collisions(&self) -> &[Collision] {
        &self.collisions
    }
    pub fn overflow(&self) -> usize {
        self.overflow
    }
    pub fn checked(&self) -> usize {
        self.checked
    }
}

impl fmt::Debug for ApplyBackPreflight {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApplyBackPreflight")
            .field("checked", &self.checked)
            .field("collisions", &self.collisions.len())
            .field("overflow", &self.overflow)
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PreflightError {
    #[error("the plan facts could not be validated")]
    Plan(#[from] PlanError),
    #[error("the repository state could not be read")]
    Git(#[from] GitInventoryError),
    #[error("a touched path could not be read")]
    Io,
    #[error("a touched path exceeds the size bound")]
    Oversize,
    #[error("the preflight was cancelled")]
    Cancelled,
    #[error("the preflight exceeded its deadline")]
    DeadlineExceeded,
}

#[cfg(unix)]
pub async fn preflight_apply_back(
    input: &PreflightInput<'_>,
    cancellation: &CancellationToken,
    bounds: PreflightBounds,
) -> Result<ApplyBackPreflight, PreflightError> {
    let deadline = Instant::now()
        .checked_add(bounds.deadline)
        .unwrap_or_else(|| Instant::now() + DEFAULT_DELTA_DEADLINE);
    if cancellation.is_cancelled() {
        return Err(PreflightError::Cancelled);
    }
    let mut paths = BTreeSet::new();
    for delta in input.accepted {
        paths.extend(delta.delta().entries().iter().map(|entry| entry.path()));
    }
    let git_inputs = input
        .service
        .read_snapshot_inputs_until(
            input.repository,
            SnapshotSelection::CurrentWorkingContent,
            cancellation,
            tokio::time::Instant::from_std(deadline),
        )
        .await?;
    let root_fd =
        open_root_fd(input.repository.root().as_os_str()).map_err(|_| PreflightError::Io)?;
    let mut baselines: HashMap<&str, (&Sha256Digest, bool)> = HashMap::new();
    for entry in input.baseline.entries() {
        let executable = entry.mode() == SnapshotFileMode::Executable;
        baselines.insert(entry.path().as_str(), (entry.content_sha256(), executable));
    }
    let mut current_index = HashMap::new();
    for entry in &git_inputs.index_entries {
        if let Some(path) = entry.path.to_str() {
            current_index.insert(path, entry);
        }
    }
    let mut current_status = HashMap::new();
    for status in &git_inputs.status {
        if let Some(path) = status.path().to_str() {
            current_status.insert(path, status);
        }
    }
    let (mut state, mut collisions, mut overflow) = (ReadState::default(), Vec::new(), 0);
    let checked = paths.len();
    for path in paths {
        let kind = classify_worktree(
            root_fd.as_raw_fd(),
            path,
            baselines.get(path).copied(),
            &mut state,
            cancellation,
            deadline,
        )?;
        let kind = kind.or_else(|| classify_index(input, path, &current_index, &current_status));
        let Some(kind) = kind else {
            continue;
        };
        if collisions.len() < MAX_ASSIGNED_PATHS {
            collisions.push(Collision {
                path: path.to_owned(),
                kind,
            });
        } else {
            overflow += 1;
        }
    }
    Ok(ApplyBackPreflight {
        checked,
        collisions,
        overflow,
    })
}

#[cfg(not(unix))]
pub async fn preflight_apply_back(
    _input: &PreflightInput<'_>,
    _cancellation: &CancellationToken,
    _bounds: PreflightBounds,
) -> Result<ApplyBackPreflight, PreflightError> {
    Err(PreflightError::Io)
}

#[cfg(unix)]
fn classify_index(
    input: &PreflightInput<'_>,
    path: &str,
    current_index: &HashMap<&str, &SnapshotIndexInput>,
    current_status: &HashMap<&str, &GitPathStatus>,
) -> Option<CollisionKind> {
    let live_index = current_index.get(path).copied();
    let plan_entry = input.plan.index_entry(path);
    match (live_index, plan_entry) {
        (Some(current), Some(plan)) => {
            let plan_mode = plan.mode.parse::<u32>().ok();
            if plan_mode != Some(current.mode)
                || current.object_id.as_str() != plan.object_id.as_str()
                || current.stage != plan.stage
                || current.intent_to_add != plan.intent_to_add
            {
                return Some(CollisionKind::IndexStagedDifferent);
            }
        }
        (Some(_), None) | (None, Some(_)) => return Some(CollisionKind::IndexStagedDifferent),
        (None, None) => {}
    }
    let status = current_status.get(path).copied()?;
    let staged_at_capture = match input.plan.status_fact(path) {
        Some(fact) => fact.staged || fact.staged_added,
        None => false,
    };
    if status.conflicted() || ((status.staged() || status.staged_added()) && !staged_at_capture) {
        return Some(CollisionKind::IndexStagedDifferent);
    }
    None
}
