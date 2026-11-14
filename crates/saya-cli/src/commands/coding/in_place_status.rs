//! The in-place facts `saya job show` reports: the edit-time baseline, what the
//! job recorded about its own changes, the finalize acknowledgement, and the
//! changed paths. A Git job has none of these keys, so its report is unchanged.
//!
//! The changed paths are the finalize-time list when the outcome exists; a
//! running job has no outcome yet, so they are computed from the before-image
//! store and the live folder with the same bound and truncation flag a managed
//! job uses.

use super::{
    CodingApplication, CodingError, context, in_place_changes, readiness_direct,
    review_in_place::MAX_FINALIZE_BYTES, root_checkout::InPlaceRootRecord,
};
use saya_harness::coding::{
    local::{JobBaseline, LocalBaseline},
    snapshot::MAX_SNAPSHOT_MANIFEST_BYTES,
    worktree::{InPlaceRootInput, verify_in_place_root},
};
use saya_store::CodingJobRecord;
use saya_types::JobId;
use serde_json::{Value, json};
use std::path::Path;

/// The in-place fields for one job, or `None` for a Git job.
pub(super) fn view(
    application: &CodingApplication,
    job: &JobId,
    record: &CodingJobRecord,
    outcome: Option<&Value>,
) -> Result<Option<Value>, CodingError> {
    if !application.is_in_place() {
        return Ok(None);
    }
    let records = application.state_root.join(job.as_str());
    let root = InPlaceRootRecord::read(&records.join("inplace-root.json"))?;
    if root.root_id() != &record.repository_id || root.snapshot_id() != &record.snapshot_id {
        return Err(CodingError::NotOwned);
    }
    let baseline = baseline(&root, &records)?;
    let (live_paths, live_truncated, recorded, overflowed) = live(application, &root, &records);
    let (changed_paths, changed_paths_truncated) = match stored_changes(outcome) {
        Some((paths, truncated)) => (paths, truncated),
        None => (live_paths, live_truncated),
    };
    Ok(Some(json!({
        "mode": "in_place",
        "baseline": {
            "root_path": baseline.root_path(),
            "root_id": baseline.repository_id(),
            "snapshot_id": baseline.snapshot_id(),
            "plan_sha256": baseline.manifest_sha256(),
            "kind": "edit_time",
        },
        "recorded": recorded,
        "changed_paths": changed_paths,
        "changed_paths_truncated": changed_paths_truncated || overflowed,
        "finalized": finalized(&records),
    })))
}

/// The job's fixed baseline, authenticated by the in-place record's plan
/// digest. A stored Git plan here is a shape error, never a fallback.
fn baseline(root: &InPlaceRootRecord, records: &Path) -> Result<LocalBaseline, CodingError> {
    let bytes = context::read(&records.join("baseline.json"), MAX_SNAPSHOT_MANIFEST_BYTES)?;
    match JobBaseline::from_verified_bytes(&bytes, root.plan_sha256())
        .map_err(CodingError::owner)?
    {
        JobBaseline::InPlace(baseline) => Ok(baseline),
        JobBaseline::Git(_) => Err(CodingError::NotOwned),
    }
}

/// The changes the job's own finalize recorded, or `None` while the job is
/// still running and has written no outcome.
fn stored_changes(outcome: Option<&Value>) -> Option<(Vec<Value>, bool)> {
    let outcome = outcome?;
    let paths = outcome.get("changed_paths")?.as_array()?.clone();
    let truncated = outcome
        .get("changed_paths_truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Some((paths, truncated))
}

/// The live changed paths and what the before-image store recorded. A root
/// that cannot be verified, or a store that cannot be read, reports no paths
/// rather than failing the whole status read; the outcome states them at
/// finalize.
fn live(
    application: &CodingApplication,
    root: &InPlaceRootRecord,
    records: &Path,
) -> (Vec<Value>, bool, Value, bool) {
    let empty = in_place_changes::recorded_facts(&[], false);
    let input = InPlaceRootInput {
        artifacts: &application.artifacts,
        state_root: &application.state_root,
        root_id: root.root_id(),
        root_path: Path::new(root.root_path()),
        snapshot_id: root.snapshot_id(),
        plan_sha256: root.plan_sha256(),
    };
    let Ok(verified) = verify_in_place_root(&input) else {
        return (Vec::new(), false, empty, false);
    };
    let Ok(live) = in_place_changes::live(&verified, records) else {
        return (Vec::new(), false, empty, false);
    };
    let (paths, truncated) = readiness_direct::changed_paths(&live.changes);
    (
        paths,
        truncated,
        in_place_changes::recorded_facts(&live.changes, live.overflowed),
        live.overflowed,
    )
}

/// Whether an in-place apply has acknowledged the reviewed folder. The
/// outcome states the acknowledgement at finalize; an apply after that writes
/// `finalize.json`, which is what this reads.
fn finalized(records: &Path) -> bool {
    let Ok(bytes) = context::read(&records.join("finalize.json"), MAX_FINALIZE_BYTES) else {
        return false;
    };
    serde_json::from_slice::<Value>(&bytes).is_ok_and(|value| value["mode"] == "in_place")
}
