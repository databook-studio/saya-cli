use super::{CodingError, NativeCodingJob, review::ReviewCheck};
use saya_harness::coding::checks::{
    CheckDiagnostics, DiagnosticDelta, DiagnosticEvidenceError, MAX_DIAGNOSTIC_BASELINE_BYTES,
    PersistedDiagnosticBaseline,
};
use saya_types::ManagedCheckoutId;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const BASELINE_PREFIX: &str = "checks-diagnostic-baseline-";

#[derive(Debug)]
pub(super) enum DiagnosticDeltaState {
    BaselinePending {
        path: PathBuf,
        record: PersistedDiagnosticBaseline,
    },
    Available(DiagnosticDelta),
    Unavailable(&'static str),
}

impl DiagnosticDeltaState {
    pub(super) fn preview(&self) -> Value {
        match self {
            Self::BaselinePending { .. } => {
                json!({"available":false,"reason":"baseline_minted_for_future_review"})
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
        let mut states = Vec::with_capacity(checks.len());
        for check in checks {
            let Some(evidence) = check.diagnostics() else {
                states.push(None);
                continue;
            };
            let path = baseline_path(
                &self.records,
                self.job_id(),
                checkout,
                evidence.receipt().profile_id(),
            )?;
            let state = if path.exists() {
                self.compare_stored_baseline(evidence, &path, &ownership)
                    .await
            } else {
                self.mint_baseline(evidence, &path, &ownership).await
            };
            states.push(Some(state));
        }
        Ok(states)
    }

    async fn mint_baseline(
        &self,
        evidence: &CheckDiagnostics,
        path: &Path,
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
            record,
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
        }
    }
}

pub(super) fn commit_pending(states: &[Option<DiagnosticDeltaState>]) -> Result<(), CodingError> {
    let mut written = Vec::new();
    for state in states.iter().filter_map(Option::as_ref) {
        let DiagnosticDeltaState::BaselinePending { path, record } = state else {
            continue;
        };
        let bytes = record.to_bytes().map_err(|_| CodingError::InvalidRequest)?;
        if let Err(error) = super::context::write_new(path, &bytes, MAX_DIAGNOSTIC_BASELINE_BYTES) {
            for path in written {
                let _ = std::fs::remove_file(path);
            }
            return Err(error);
        }
        written.push(path.clone());
    }
    Ok(())
}

pub(super) fn summary(states: &[Option<DiagnosticDeltaState>]) -> Value {
    let states = states.iter().filter_map(Option::as_ref).collect::<Vec<_>>();
    if states.is_empty() {
        return json!({"available":false,"reason":"comparable_baseline_not_minted"});
    }
    if states.iter().all(|state| state.available().is_some()) {
        let deltas = states
            .iter()
            .filter_map(|state| state.available())
            .collect::<Vec<_>>();
        return json!({
            "available":true,
            "introduced_count":deltas.iter().map(|delta|delta.introduced().len()).sum::<usize>(),
            "unchanged_count":deltas.iter().map(|delta|delta.unchanged().len()).sum::<usize>(),
            "resolved_count":deltas.iter().map(|delta|delta.resolved().len()).sum::<usize>()
        });
    }
    let reason = states
        .iter()
        .find_map(|state| match state {
            DiagnosticDeltaState::BaselinePending { .. } => {
                Some("baseline_minted_for_future_review")
            }
            DiagnosticDeltaState::Unavailable(reason) => Some(*reason),
            _ => None,
        })
        .unwrap_or("baseline_unavailable");
    json!({"available":false,"reason":reason})
}

fn entries(entries: &[saya_harness::coding::checks::Diagnostic]) -> Vec<Value> {
    entries
        .iter()
        .map(|entry| {
            json!({
                "severity":format!("{:?}",entry.severity()),
                "code":entry.code(),
                "message":saya_types::redact(entry.message()),
                "locations":entry.locations().iter().map(|location|json!({
                    "path":location.path(),
                    "line":location.line_start(),
                    "column":location.column_start()
                })).collect::<Vec<_>>()
            })
        })
        .collect()
}
