//! The private files a fresh job writes before it can run: its request, its
//! baseline — and, in place, the start snapshot a later review compares
//! against — so a resume reads what this job started as rather than what the
//! folder holds now.

use super::super::{CodingError, CodingJobRequest, context, in_place_start, root_checkout};
use saya_agent::CancellationToken;
use saya_harness::coding::{git::GitService, local::JobBaseline};
use std::path::Path;

/// The request is the job's whole approved shape; the plan is the exact
/// baseline it was approved over. Both are written once, into the job's own
/// private records directory.
pub(super) async fn write(
    request: &CodingJobRequest,
    plan: &JobBaseline,
    records: &Path,
    service: &GitService,
    token: &CancellationToken,
) -> Result<(), CodingError> {
    context::write_new(
        &records.join("request.json"),
        &serde_json::to_vec(&request).map_err(CodingError::owner)?,
        64 * 1024,
    )?;
    match plan {
        JobBaseline::Git(_) => {
            context::write_new(
                &records.join("plan.json"),
                plan.canonical_bytes(),
                saya_harness::coding::snapshot::MAX_SNAPSHOT_MANIFEST_BYTES,
            )?;
        }
        JobBaseline::InPlace(baseline) => {
            context::write_new(
                &records.join("baseline.json"),
                plan.canonical_bytes(),
                saya_harness::coding::snapshot::MAX_SNAPSHOT_MANIFEST_BYTES,
            )?;
            // Shell-made changes are this start read's difference; non-Git is unrecorded.
            let start =
                in_place_start::capture(service, Path::new(baseline.root_path()), records, token)
                    .await?;
            let (git, start_head) = in_place_start::git_and_head(&start);
            root_checkout::InPlaceRootRecord::new(baseline, git, start_head)
                .write(&records.join("inplace-root.json"))?;
        }
    }
    Ok(())
}
