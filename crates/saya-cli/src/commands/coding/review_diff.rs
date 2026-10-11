use super::{CodingError, NativeCodingJob, context, create::digest};
use saya_harness::{
    coding::{
        integration::{CheckoutDelta, DeltaKind},
        snapshot::CapturedSnapshot,
        worktree::VerifiedCheckout,
    },
    workspace::Workspace,
};
use serde_json::{Value, json};
use std::path::Path;
impl NativeCodingJob {
    pub(super) fn review_diff(
        &self,
        candidate: &VerifiedCheckout,
        baseline: &CapturedSnapshot,
        delta: &CheckoutDelta,
    ) -> Result<Value, CodingError> {
        let workspace =
            Workspace::open(Path::new(candidate.path().as_str())).map_err(CodingError::owner)?;
        let mut previews = Vec::new();
        let mut remaining = 64 * 1024;
        for entry in delta.entries().iter().take(128) {
            if remaining == 0 {
                break;
            }
            let old = baseline
                .entries()
                .iter()
                .find(|old| old.path().as_str() == entry.path());
            let before = if let Some(old) = old.filter(|old| old.captured_bytes() <= 16 * 1024) {
                let bytes = context::read(
                    &self
                        .application
                        .artifacts
                        .root()
                        .join(baseline.snapshot_id().as_str())
                        .join("objects")
                        .join(old.content_sha256().as_str()),
                    16 * 1024,
                )?;
                if digest(&bytes)? != *old.content_sha256() {
                    return Err(CodingError::StaleReview);
                }
                Some(String::from_utf8_lossy(&bytes).into_owned())
            } else {
                None
            };
            let after = if entry.kind() != DeltaKind::Deleted {
                let read = workspace
                    .read(entry.path(), (remaining.min(16 * 1024)) as u64)
                    .map_err(CodingError::owner)?;
                if entry
                    .content_sha256()
                    .is_none_or(|expected| expected.as_str() != read.digest)
                {
                    return Err(CodingError::StaleReview);
                }
                Some(String::from_utf8_lossy(&read.bytes).into_owned())
            } else {
                None
            };
            let size =
                before.as_ref().map_or(0, String::len) + after.as_ref().map_or(0, String::len);
            if size > remaining {
                break;
            }
            remaining -= size;
            previews.push(json!({"path":entry.path(),"before":before.map(|value|saya_types::redact(&value)),"after":after.map(|value|saya_types::redact(&value)),"content_complete":entry.bytes()<=16*1024&&old.is_none_or(|old|old.captured_bytes()<=16*1024)}));
        }
        Ok(
            json!({"files":previews,"total_files":delta.len(),"bounded":true,"format":"before_after"}),
        )
    }
}
