use super::{CodingError, NativeCodingJob, review::ReviewCheck};
use saya_harness::coding::checks::{
    CheckDiagnostics, DiagnosticDelta, DiagnosticEvidenceError, MAX_DIAGNOSTIC_BASELINE_BYTES,
    PersistedDiagnosticBaseline,
};
use saya_types::ManagedCheckoutId;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

mod publication;
mod summary;

pub(super) use publication::commit_pending;
use summary::entries;
pub(super) use summary::summary;

const BASELINE_PREFIX: &str = "checks-diagnostic-baseline-";

#[derive(Debug)]
pub(super) enum DiagnosticDeltaState {
    BaselinePending {
        path: PathBuf,
        publication: PathBuf,
        record: PersistedDiagnosticBaseline,
        committed: bool,
    },
    Available(DiagnosticDelta),
    Unavailable(&'static str),
}

impl DiagnosticDeltaState {
    pub(super) fn preview(&self) -> Value {
        match self {
            Self::BaselinePending { committed, .. } => {
                let reason = if *committed {
                    "baseline_minted_for_future_review"
                } else {
                    "baseline_pending_publication"
                };
                json!({"available":false,"reason":reason})
            }
            Self::Unavailable(reason) => json!({"available":false,"reason":reason}),
            Self::Available(delta) => json!({
                "available":true,
                "baseline_source_sha256":delta.baseline_source(),
                "current_source_sha256":delta.current_source(),
                "baseline_outcome":delta.baseline_outcome(),
                "current_outcome":delta.current_outcome(),
                "current_successful":delta.current_successful(),
                "introduced_count":delta.introduced().len(),
                "unchanged_count":delta.unchanged().len(),
                "resolved_count":delta.resolved().len(),
                "introduced":entries(delta.introduced()),
                "unchanged":entries(delta.unchanged()),
                "resolved":entries(delta.resolved())
            }),
        }
    }

    fn available(&self) -> Option<&DiagnosticDelta> {
        match self {
            Self::Available(delta) => Some(delta),
            _ => None,
        }
    }

    fn pending(&self) -> bool {
        matches!(self, Self::BaselinePending { .. })
    }
}

pub(super) fn baseline_path(
    records: &Path,
    job: &saya_types::JobId,
    checkout: &ManagedCheckoutId,
    profile: &str,
) -> Result<PathBuf, CodingError> {
    let key = format!("{}\n{}\n{}", job.as_str(), checkout.as_str(), profile);
    let digest = super::create::digest(key.as_bytes())?;
    Ok(records.join(format!("{BASELINE_PREFIX}{}.jsonl", digest.as_str())))
}

impl NativeCodingJob {
    pub(super) async fn prepare_diagnostic_deltas(
        &self,
        checkout: &ManagedCheckoutId,
        checks: &[ReviewCheck],
    ) -> Result<Vec<Option<DiagnosticDeltaState>>, CodingError> {
        let ownership = self.ownership(checkout);
        let paths = checks
            .iter()
            .filter_map(ReviewCheck::diagnostics)
            .map(|evidence| {
                baseline_path(
                    &self.records,
                    self.job_id(),
                    checkout,
                    evidence.receipt().profile_id(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        if paths.is_empty() {
            return Ok(checks.iter().map(|_| None).collect());
        }
        let publication = publication::publication_path(&self.records, &paths)?;
        let published = if publication.exists() {
            publication::read_publication(&publication, &paths)
        } else {
            None
        };
        let mut states = Vec::with_capacity(checks.len());
        let mut path_index = 0;
        for check in checks {
            let Some(evidence) = check.diagnostics() else {
                states.push(None);
                continue;
            };
            let path = &paths[path_index];
            path_index += 1;
            let state = if path.exists() || publication.exists() {
                if published.is_none() {
                    DiagnosticDeltaState::Unavailable("baseline_unavailable")
                } else {
                    self.compare_stored_baseline(evidence, path, &ownership)
                        .await
                }
            } else {
                self.mint_baseline(evidence, path, &publication, &ownership)
                    .await
            };
            states.push(Some(state));
        }
        Ok(states)
    }

    async fn mint_baseline(
        &self,
        evidence: &CheckDiagnostics,
        path: &Path,
        publication: &Path,
        ownership: &saya_harness::coding::worktree::OwnershipInput<'_>,
    ) -> DiagnosticDeltaState {
        let minted = match evidence.baseline(ownership, &self.cancellation).await {
            Ok(minted) => minted,
            Err(DiagnosticEvidenceError::Incomplete) => {
                return DiagnosticDeltaState::Unavailable("baseline_incomplete");
            }
            Err(DiagnosticEvidenceError::Incomparable) => {
                return DiagnosticDeltaState::Unavailable("baseline_incomparable");
            }
            Err(DiagnosticEvidenceError::Execution(_)) => {
                return DiagnosticDeltaState::Unavailable("baseline_unavailable");
            }
            // `#[non_exhaustive]`: a future variant leaves the baseline
            // unavailable rather than minting from incomplete evidence.
            Err(_) => return DiagnosticDeltaState::Unavailable("baseline_unavailable"),
        };
        let record = match evidence.persisted_baseline(&minted) {
            Ok(record) => record,
            Err(_) => return DiagnosticDeltaState::Unavailable("baseline_unavailable"),
        };
        if record
            .to_bytes()
            .map(|bytes| bytes.len() <= MAX_DIAGNOSTIC_BASELINE_BYTES)
            != Ok(true)
        {
            return DiagnosticDeltaState::Unavailable("baseline_oversized");
        }
        DiagnosticDeltaState::BaselinePending {
            path: path.to_owned(),
            publication: publication.to_owned(),
            record,
            committed: false,
        }
    }

    async fn compare_stored_baseline(
        &self,
        evidence: &CheckDiagnostics,
        path: &Path,
        ownership: &saya_harness::coding::worktree::OwnershipInput<'_>,
    ) -> DiagnosticDeltaState {
        let bytes = match super::context::read(path, MAX_DIAGNOSTIC_BASELINE_BYTES) {
            Ok(bytes) => bytes,
            Err(_) => return DiagnosticDeltaState::Unavailable("baseline_unavailable"),
        };
        let baseline = match PersistedDiagnosticBaseline::from_bytes(&bytes) {
            Ok(baseline) => baseline,
            Err(_) => return DiagnosticDeltaState::Unavailable("baseline_unavailable"),
        };
        match evidence
            .compare_persisted_baseline(&baseline, ownership, &self.cancellation)
            .await
        {
            Ok(delta) => DiagnosticDeltaState::Available(delta),
            Err(DiagnosticEvidenceError::Incomparable) => {
                DiagnosticDeltaState::Unavailable("baseline_incomparable")
            }
            Err(DiagnosticEvidenceError::Incomplete) => {
                DiagnosticDeltaState::Unavailable("baseline_unavailable")
            }
            Err(DiagnosticEvidenceError::Execution(_)) => {
                DiagnosticDeltaState::Unavailable("baseline_unavailable")
            }
            // `#[non_exhaustive]`: a future variant keeps the delta
            // unavailable rather than comparing against failed evidence.
            Err(_) => DiagnosticDeltaState::Unavailable("baseline_unavailable"),
        }
    }
}
