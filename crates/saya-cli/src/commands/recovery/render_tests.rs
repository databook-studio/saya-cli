//! Tests for the recovery renderers: every sentence pinned, no digests or
//! file content outside the bounded diff text.

use super::*;
use saya_types::{
    CheckpointState, CheckpointUsage, EditSource, FileVersion, MAX_CHECKPOINT_SESSION_BYTES,
    MAX_CHECKPOINTS_PER_SESSION, RestoreEligibility,
};

fn item(n: u64, path: &str, source: EditSource, state: CheckpointState) -> ChangeItem {
    ChangeItem {
        id: format!("ck-{:032x}", n),
        path: path.to_owned(),
        source,
        state,
        before: BeforeSummary::Present { size: 4 },
        after: Some(FileVersion {
            sha256: format!("{:064x}", n),
            size: 6,
        }),
        created_unix_ms: 10_000,
        restore: StatusNow::Known(RestoreEligibility::Eligible),
    }
}

#[cfg(test)]
#[path = "render_tests/diff.rs"]
mod diff;
#[cfg(test)]
#[path = "render_tests/listing.rs"]
mod listing;
#[cfg(test)]
#[path = "render_tests/restore.rs"]
mod restore;
