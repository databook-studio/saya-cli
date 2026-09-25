//! At a step's completion, its declared deliverables become the artifact
//! manifest: every output the step's `expects` declared is resolved against
//! the run workspace — an existing file recorded with its size and sha256
//! digest, a declared deliverable the step never produced recorded as
//! missing, and a name the workspace refuses (an escape, a link, a denied
//! name, an over-bound file) refusing the recording outright. Only a
//! missing file resolves to a missing deliverable; every other error is a
//! refusal — reported, never silent, never guessed at.
//!
//! The resolution goes through the same contained primitives everything
//! else uses: [`manifest::entry`] is the walk's scan-and-digest discipline
//! for one file, so containment applies to a deliverable exactly as it does
//! to any workspace read.

use saya_types::{Deliverable, StepSpec};

use crate::{HarnessError, workspace::Workspace, workspace::manifest};

use super::ManifestBounds;

/// Resolves the step's declared deliverables, in declaration order, against
/// the workspace and the manifest bounds the briefs use.
pub(super) fn resolve(
    workspace: &Workspace,
    spec: &StepSpec,
    bounds: &ManifestBounds,
) -> Result<Vec<Deliverable>, HarnessError> {
    let mut entries = Vec::with_capacity(spec.expects.len());
    for hint in &spec.expects {
        let deliverable = match manifest::entry(workspace, &hint.name, bounds.max_file_bytes) {
            Ok(entry) => Deliverable::present(&hint.name, entry.size, entry.digest),
            Err(error) if error.is_not_found() => Deliverable::missing(&hint.name),
            Err(source) => return Err(source),
        };
        entries.push(deliverable);
    }
    Ok(entries)
}
