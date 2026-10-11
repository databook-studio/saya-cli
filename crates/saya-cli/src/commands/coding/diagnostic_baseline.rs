use super::{CodingError, NativeCodingJob, review::ReviewCheck};
use saya_harness::coding::checks::{
    CheckDiagnostics, DiagnosticDelta, DiagnosticEvidenceError, MAX_DIAGNOSTIC_BASELINE_BYTES,
    PersistedDiagnosticBaseline,
};
use saya_types::ManagedCheckoutId;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const BASELINE_PREFIX: &str = "checks-diagnostic-baseline-";
const PUBLICATION_PREFIX: &str = "checks-diagnostic-baseline-commit-";
const PUBLICATION_VERSION: u32 = 1;
const MAX_PUBLICATION_ENTRIES: usize = 128;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BaselinePublication {
    version: u32,
    baselines: Vec<BaselinePublicationEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BaselinePublicationEntry {
    path: String,
    bytes: usize,
    sha256: saya_types::Sha256Digest,
}

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
        let publication = publication_path(&self.records, &paths)?;
        let published = if publication.exists() {
            read_publication(&publication, &paths)
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
                    self.compare_stored_baseline(evidence, &path, &ownership)
                        .await
                }
            } else {
                self.mint_baseline(evidence, &path, &publication, &ownership)
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
        }
    }
}

fn publication_path(records: &Path, paths: &[PathBuf]) -> Result<PathBuf, CodingError> {
    if paths.is_empty() || paths.len() > MAX_PUBLICATION_ENTRIES {
        return Err(CodingError::InvalidRequest);
    }
    let mut names = paths
        .iter()
        .map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .ok_or(CodingError::InvalidRequest)
                .map(str::to_owned)
        })
        .collect::<Result<Vec<_>, _>>()?;
    names.sort_unstable();
    let digest = super::create::digest(names.join("\n").as_bytes())?;
    Ok(records.join(format!("{PUBLICATION_PREFIX}{}.jsonl", digest.as_str())))
}

fn read_publication(path: &Path, paths: &[PathBuf]) -> Option<BaselinePublication> {
    let bytes = super::context::read(path, MAX_DIAGNOSTIC_BASELINE_BYTES).ok()?;
    let publication: BaselinePublication = serde_json::from_slice(&bytes).ok()?;
    if publication.version != PUBLICATION_VERSION
        || publication.baselines.len() != paths.len()
        || publication.baselines.len() > MAX_PUBLICATION_ENTRIES
    {
        return None;
    }
    let mut expected = paths
        .iter()
        .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
        .collect::<Vec<_>>();
    expected.sort_unstable();
    let mut actual = publication
        .baselines
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<Vec<_>>();
    actual.sort_unstable();
    if actual != expected {
        return None;
    }
    for entry in &publication.baselines {
        let Some(path) = paths.iter().find(|path| {
            path.file_name().and_then(|name| name.to_str()) == Some(entry.path.as_str())
        }) else {
            return None;
        };
        let bytes = super::context::read(path, MAX_DIAGNOSTIC_BASELINE_BYTES).ok()?;
        if bytes.len() != entry.bytes || super::create::digest(&bytes).ok()? != entry.sha256 {
            return None;
        }
    }
    Some(publication)
}

pub(super) fn commit_pending(
    states: &mut [Option<DiagnosticDeltaState>],
) -> Result<bool, CodingError> {
    let pending = states
        .iter()
        .filter_map(Option::as_ref)
        .filter(|state| state.pending())
        .count();
    let comparable = states.iter().filter_map(Option::as_ref).count();
    if pending == 0 || pending != comparable {
        return Ok(false);
    }
    let publication = states
        .iter()
        .filter_map(Option::as_ref)
        .find_map(|state| match state {
            DiagnosticDeltaState::BaselinePending { publication, .. } => Some(publication.clone()),
            _ => None,
        })
        .ok_or(CodingError::InvalidRequest)?;
    let mut entries = Vec::new();
    let mut written = Vec::new();
    for state in states.iter().filter_map(Option::as_ref) {
        let DiagnosticDeltaState::BaselinePending {
            path,
            publication: state_publication,
            record,
            ..
        } = state
        else {
            return Err(CodingError::InvalidRequest);
        };
        if *state_publication != publication {
            return Err(CodingError::InvalidRequest);
        }
        let bytes = record.to_bytes().map_err(|_| CodingError::InvalidRequest)?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(CodingError::InvalidRequest)?;
        entries.push(BaselinePublicationEntry {
            path: name.to_owned(),
            bytes: bytes.len(),
            sha256: super::create::digest(&bytes)?,
        });
        if let Err(error) = super::context::write_new(path, &bytes, MAX_DIAGNOSTIC_BASELINE_BYTES) {
            for path in written {
                let _ = std::fs::remove_file(path);
            }
            return Err(error);
        }
        written.push(path.clone());
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    let marker = BaselinePublication {
        version: PUBLICATION_VERSION,
        baselines: entries,
    };
    let bytes = serde_json::to_vec(&marker).map_err(CodingError::owner)?;
    let publication_absent = std::fs::symlink_metadata(&publication).is_err();
    if let Err(error) =
        super::context::write_new(&publication, &bytes, MAX_DIAGNOSTIC_BASELINE_BYTES)
    {
        if publication_absent {
            let _ = std::fs::remove_file(&publication);
        }
        for path in written {
            let _ = std::fs::remove_file(path);
        }
        return Err(error);
    }
    for state in states.iter_mut().filter_map(Option::as_mut) {
        if let DiagnosticDeltaState::BaselinePending { committed, .. } = state {
            *committed = true;
        }
    }
    Ok(true)
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
            DiagnosticDeltaState::BaselinePending { committed, .. } => Some(if *committed {
                "baseline_minted_for_future_review"
            } else {
                "baseline_pending_publication"
            }),
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
