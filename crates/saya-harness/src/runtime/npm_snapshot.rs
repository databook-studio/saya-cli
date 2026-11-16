//! Bounded read-only source copies and their exact filtered input digest.

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use saya_agent::CancellationToken;
use saya_types::Sha256Digest;

use crate::{
    coding::{
        git::IgnoredPaths,
        integration::{CandidateError, DeltaError},
    },
    workspace::Workspace,
};

use super::npm::{NpmCheckSource, NpmRuntimeError};

mod copy;

const MAX_SOURCE_FILES: usize = 1_000;
const MAX_SOURCE_ENTRIES: usize = 2_000;
const MAX_SOURCE_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_SOURCE_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
pub(super) const SOURCE_SNAPSHOT_POLICY_ID: &str = "saya-node-source-snapshot-v1-64mib";

pub(super) struct SourceSnapshot {
    pub(super) path: PathBuf,
    pub(super) sha256: String,
}

pub(super) fn copy_source(
    workspace: &Workspace,
    source: NpmCheckSource,
    destination: &Path,
    expected_source_sha256: &str,
    cancellation: &CancellationToken,
    timeout: Duration,
) -> Result<SourceSnapshot, NpmRuntimeError> {
    let started = Instant::now();
    let deadline = started + timeout;
    check_active(cancellation, deadline)?;
    let actual = source_digest(source, workspace, cancellation, remaining(deadline))?;
    check_active(cancellation, deadline)?;
    if actual.as_str() != expected_source_sha256 {
        return Err(NpmRuntimeError::SourceChanged);
    }
    let snapshot_sha256 = copy::copy_tree(workspace, destination, cancellation, deadline)?;
    check_active(cancellation, deadline)?;
    let after = source_digest(source, workspace, cancellation, remaining(deadline))?;
    check_active(cancellation, deadline)?;
    if after.as_str() != expected_source_sha256 {
        return Err(NpmRuntimeError::SourceChanged);
    }
    Ok(SourceSnapshot {
        path: destination.to_path_buf(),
        sha256: snapshot_sha256,
    })
}

fn source_digest(
    source: NpmCheckSource,
    workspace: &Workspace,
    cancellation: &CancellationToken,
    deadline: Duration,
) -> Result<Sha256Digest, NpmRuntimeError> {
    let ignored = IgnoredPaths::none();
    let digest = match source {
        NpmCheckSource::ManagedCandidate => {
            crate::coding::integration::managed_candidate_source_digest(
                workspace.root(),
                &ignored,
                cancellation,
                deadline,
            )
        }
        NpmCheckSource::OperatorProject => {
            crate::coding::integration::operator_project_source_digest(
                workspace.root(),
                &ignored,
                cancellation,
                deadline,
            )
        }
    };
    digest.map_err(map_candidate_error)
}

fn map_candidate_error(error: CandidateError) -> NpmRuntimeError {
    match error {
        CandidateError::Walk(DeltaError::Cancelled) => NpmRuntimeError::Cancelled,
        CandidateError::Walk(DeltaError::DeadlineExceeded) => NpmRuntimeError::PreparationTimedOut,
        _ => NpmRuntimeError::InvalidProject,
    }
}

fn check_active(
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<(), NpmRuntimeError> {
    if cancellation.is_cancelled() {
        Err(NpmRuntimeError::Cancelled)
    } else if Instant::now() >= deadline {
        Err(NpmRuntimeError::PreparationTimedOut)
    } else {
        Ok(())
    }
}

fn remaining(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now())
}
