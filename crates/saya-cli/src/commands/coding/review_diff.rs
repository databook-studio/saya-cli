use super::{CodingError, NativeCodingJob, RootChange, context, create::digest};
use saya_harness::{
    coding::{
        integration::DeltaKind,
        local::before_image::{BeforeImageStore, BeforeState},
        snapshot::{CapturedSnapshot, SnapshotArtifacts},
        worktree::VerifiedCheckout,
    },
    workspace::Workspace,
};
use serde_json::{Value, json};
use std::path::Path;

/// Where the "before" side of a review preview comes from: a managed job's
/// captured snapshot, or an in-place job's before-image store.
pub(super) enum BaselineSource<'a> {
    Snapshot(&'a CapturedSnapshot),
    BeforeImages(&'a BeforeImageStore),
}

impl BaselineSource<'_> {
    /// The old text for one path and whether the whole old file is shown. A
    /// stored object whose digest no longer matches is stale evidence.
    fn before(
        &self,
        artifacts: &SnapshotArtifacts,
        path: &str,
    ) -> Result<(Option<String>, bool), CodingError> {
        match self {
            Self::Snapshot(baseline) => {
                let old = baseline
                    .entries()
                    .iter()
                    .find(|old| old.path().as_str() == path);
                let Some(old) = old else {
                    return Ok((None, true));
                };
                if old.captured_bytes() > 16 * 1024 {
                    return Ok((None, false));
                }
                let bytes = context::read(
                    &artifacts
                        .root()
                        .join(baseline.snapshot_id().as_str())
                        .join("objects")
                        .join(old.content_sha256().as_str()),
                    16 * 1024,
                )?;
                if digest(&bytes)? != *old.content_sha256() {
                    return Err(CodingError::StaleReview);
                }
                Ok((Some(String::from_utf8_lossy(&bytes).into_owned()), true))
            }
            Self::BeforeImages(store) => {
                let entries = store.entries().map_err(CodingError::owner)?;
                let Some(entry) = entries.iter().find(|entry| entry.path == path) else {
                    return Ok((None, true));
                };
                match &entry.state {
                    BeforeState::File { size, .. } if *size <= 16 * 1024 => {
                        let bytes = store
                            .read_image(entry)
                            .map_err(CodingError::owner)?
                            .unwrap_or_default();
                        Ok((Some(String::from_utf8_lossy(&bytes).into_owned()), true))
                    }
                    BeforeState::File { .. } => Ok((None, false)),
                    // A path first seen absent or deliberately unrecorded has
                    // no old text to show.
                    BeforeState::Absent | BeforeState::Unrecorded(_) => Ok((None, true)),
                }
            }
        }
    }
}

impl NativeCodingJob {
    /// A bounded before/after preview over the job's changed paths. Only the
    /// paths the review already bound are read, and the current bytes must
    /// still match the change's digest; a mismatch is stale evidence.
    pub(super) fn review_diff(
        &self,
        candidate: &VerifiedCheckout,
        source: BaselineSource<'_>,
        changes: &[RootChange],
    ) -> Result<Value, CodingError> {
        let workspace =
            Workspace::open(Path::new(candidate.path().as_str())).map_err(CodingError::owner)?;
        let mut previews = Vec::new();
        let mut remaining = 64 * 1024;
        for change in changes.iter().take(128) {
            if remaining == 0 {
                break;
            }
            let (before, before_complete) =
                source.before(&self.application.artifacts, &change.path)?;
            let after = if change.kind != DeltaKind::Deleted && change.content_sha256.is_some() {
                match workspace.read(&change.path, (remaining.min(16 * 1024)) as u64) {
                    Ok(read) => {
                        if change
                            .content_sha256
                            .as_ref()
                            .is_some_and(|expected| expected.as_str() != read.digest)
                        {
                            return Err(CodingError::StaleReview);
                        }
                        Some(String::from_utf8_lossy(&read.bytes).into_owned())
                    }
                    // A path this workspace refuses to read (over the bound, a
                    // symlink) has no preview; the change itself is still
                    // reviewed.
                    Err(_) => None,
                }
            } else {
                None
            };
            let size =
                before.as_ref().map_or(0, String::len) + after.as_ref().map_or(0, String::len);
            if size > remaining {
                break;
            }
            remaining -= size;
            previews.push(json!({"path":change.path,"before":before.map(|value|saya_types::redact(&value)),"after":after.map(|value|saya_types::redact(&value)),"content_complete":change.bytes<=16*1024&&before_complete}));
        }
        Ok(
            json!({"files":previews,"total_files":changes.len(),"bounded":true,"format":"before_after"}),
        )
    }
}
