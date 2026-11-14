//! The start snapshot an in-place job keeps: whether its folder is a Git
//! working tree, the HEAD it held, and every path dirty then.
//!
//! Shell-made changes are the status difference between this snapshot and the
//! review, so the snapshot is written once, when the job is created, and never
//! rewritten by a resume. A folder that is not a Git repository, or whose Git
//! cannot be read, still starts: the snapshot is `unrecorded` and review says
//! so.

use super::CodingError;
use saya_agent::CancellationToken;
use saya_harness::coding::{
    git::GitService,
    local::shell_changes::{MAX_SHELL_START_BYTES, ShellStart, capture_start},
};
use std::path::Path;
use std::time::Duration;

/// The start snapshot's file name under the job's records.
const START_FILE: &str = "git-start.json";

/// How long the start status and HEAD read may take. The listing is bounded
/// by the Git service's own output and path limits; this only stops a wedged
/// Git from delaying a start.
const START_READ_DEADLINE: Duration = Duration::from_secs(10);

/// Reads the folder's status and HEAD once, bounds the snapshot, and writes it.
/// The returned value is what the in-place record states about Git.
pub(super) async fn capture(
    service: &GitService,
    root: &Path,
    records: &Path,
    cancellation: &CancellationToken,
) -> Result<ShellStart, CodingError> {
    let start = bounded(capture_start(service, root, cancellation, START_READ_DEADLINE).await);
    let bytes = serde_json::to_vec(&start).map_err(CodingError::owner)?;
    super::context::write_new(&records.join(START_FILE), &bytes, MAX_SHELL_START_BYTES)?;
    Ok(start)
}

/// The recorded start snapshot, or `unrecorded` when it cannot be read: shell
/// changes are advisory and never fail a review.
pub(super) fn read(records: &Path) -> ShellStart {
    let Ok(bytes) = super::context::read(&records.join(START_FILE), MAX_SHELL_START_BYTES) else {
        return ShellStart::Unrecorded {
            reason: saya_harness::coding::local::shell_changes::ShellUnrecorded::GitFailed,
        };
    };
    serde_json::from_slice(&bytes).unwrap_or(ShellStart::Unrecorded {
        reason: saya_harness::coding::local::shell_changes::ShellUnrecorded::GitFailed,
    })
}

/// Whether the folder was a Git working tree at start, and the HEAD it held.
pub(super) fn git_and_head(start: &ShellStart) -> (bool, Option<String>) {
    match start {
        ShellStart::Recorded { head, .. } => (true, head.clone()),
        ShellStart::Unrecorded { .. } => (false, None),
    }
}

/// A snapshot whose serialized form would pass the stored bound is kept whole;
/// one that would not keeps its HEAD and states `truncated`, which every later
/// read reports as unrecorded rather than attributing changes to a partial
/// list.
fn bounded(start: ShellStart) -> ShellStart {
    let fits = serde_json::to_vec(&start).is_ok_and(|bytes| bytes.len() <= MAX_SHELL_START_BYTES);
    if fits {
        return start;
    }
    match start {
        ShellStart::Recorded { head, .. } => ShellStart::Recorded {
            head,
            entries: Vec::new(),
            truncated: true,
        },
        unrecorded => unrecorded,
    }
}
