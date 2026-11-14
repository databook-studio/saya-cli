//! The bounded outcome block every finished job keeps: the terminal state, the
//! model's final answer, the commits made in the working checkout since its
//! base, the changed paths, the checkout itself, and readiness as one field.
//!
//! The answer and commit subjects are model-controlled text: they are redacted,
//! bounded, and the block is marked `model_prose: untrusted`. The artifact is
//! rewritten on every finalization (a resume replaces the earlier outcome) and
//! is read back by `saya job show`; `saya start` reads the same block.

use super::{
    CodingError, NativeCodingJob, context, readiness,
    rewind::{RewindStore, store_root},
};
use saya_types::CodingRunFailure;
use saya_types::{JobId, JobState};
use serde_json::{Value, json};

/// Most bytes of the model's final answer the outcome keeps.
const MAX_ANSWER_BYTES: usize = 16 * 1024;

/// Most commits one outcome names; the page says whether more exist.
const MAX_COMMITS: usize = 32;

/// Most bytes of a failure's message the outcome keeps as the pause reason.
const MAX_REASON_BYTES: usize = 512;

/// The artifact's own bound. If the readiness facts push the block past it,
/// the changed paths are dropped and marked truncated, never the answer.
const MAX_OUTCOME_BYTES: usize = 256 * 1024;

const OUTCOME_FILE: &str = "outcome.json";

impl NativeCodingJob {
    /// The persisted outcome block, or `None` before the first finalization.
    pub(crate) fn outcome(&self) -> Option<Value> {
        read(&self.records)
    }

    /// The durable record's checkout path and base commit, used only when the
    /// live checkout no longer verifies at its base (the model committed).
    async fn recorded_root_checkout(
        &self,
    ) -> Option<(saya_types::ManagedCheckoutPath, saya_types::GitObjectId)> {
        let checkout_id = super::root_checkout::read_root_checkout(&self.records).ok()??;
        let record = self
            .application
            .store
            .get_managed_checkout(self.job_id(), &checkout_id)
            .await
            .ok()??;
        Some((
            record.ownership.managed_path().clone(),
            record.ownership.base_commit().clone(),
        ))
    }

    /// Builds and persists the outcome for this finalization. The terminal
    /// event has already been appended, so `state` is the authoritative one.
    pub(super) async fn record_outcome(
        &self,
        state: JobState,
        failure: Option<&CodingRunFailure>,
        answer: Option<&str>,
        answer_truncated: bool,
    ) -> Result<(), CodingError> {
        // A readiness computation started on the final turn's first text is
        // settled here. Its cache entry is the one this outcome would have
        // computed, so the block is identical; when the owner's preparation
        // has since moved the receipt the key no longer matches and the facts
        // are recomputed as before.
        self.settle_readiness_precompute().await;
        // The advisory facts are computed first: on a cache miss they verify W
        // once, and the commit walk below reuses that verification instead of
        // inventorying the same checkout again.
        let facts = if self.request.write_paths.is_empty() {
            None
        } else {
            Some(self.readiness_facts().await)
        };
        let readiness = self.candidate_readiness().await;
        let view = readiness.view().clone();
        let envelope = readiness::final_envelope(&readiness);
        // A checkout the model committed into no longer verifies at its base
        // commit; the durable ownership record still names the checkout and
        // its base, which is all the commit walk needs.
        let checkout = match facts.as_ref().and_then(|facts| facts.checkout.clone()) {
            Some(checkout) => Some(checkout),
            None => self.recorded_root_checkout().await,
        };
        let (commits, commits_truncated, commits_error) = match &checkout {
            Some((path, base)) => match self
                .application
                .service
                .commits_since(path, base, MAX_COMMITS, &self.cancellation)
                .await
            {
                Ok(page) => (page.commits, page.truncated, false),
                Err(_) => (Vec::new(), false, true),
            },
            None => (Vec::new(), false, false),
        };
        let checkout = checkout.as_ref().map(|(path, _)| path.as_str().to_owned());
        let commits = commits
            .into_iter()
            .map(|commit| {
                json!({
                    "sha": commit.sha,
                    "subject": commit.subject,
                    "subject_truncated": commit.subject_truncated,
                })
            })
            .collect::<Vec<_>>();
        let (answer, answer_truncated) = match answer {
            Some(answer) => {
                let (answer, bounded) = bounded(&saya_types::redact(answer), MAX_ANSWER_BYTES);
                (json!(answer), answer_truncated || bounded)
            }
            None => (Value::Null, false),
        };
        let (reason, reason_detail) = match failure {
            Some(failure) => (
                json!(failure.class.as_str()),
                failure
                    .message
                    .as_deref()
                    .map(|message| bounded(&saya_types::redact(message), MAX_REASON_BYTES).0)
                    .map_or(Value::Null, |message| json!(message)),
            ),
            None => (Value::Null, Value::Null),
        };
        let changed_paths = view.get("changed_paths").cloned().unwrap_or(json!([]));
        let changed_paths_truncated = view
            .get("changed_paths_truncated")
            .cloned()
            .unwrap_or(json!(false));
        // An in-place job's folder is the result: state its root, the fixed
        // baseline, the review acknowledgement and the undo it still has. A Git
        // outcome keeps its exact published shape.
        let in_place = if self.is_in_place() {
            let baseline = self.in_place_baseline()?;
            let record = self.in_place_record()?;
            let finalized = self.in_place_finalize()?;
            let finalize_source = finalized
                .as_ref()
                .map(|(digest, _)| digest.as_str().to_owned());
            let paths = changed_paths
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|entry| entry.get("path").and_then(Value::as_str))
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let checkpoints = RewindStore::open(&store_root(&self.records)).turns()?.len();
            // What the job's own tools recorded, from the before-image store.
            // A root that no longer verifies records nothing here; the
            // readiness view already states what it could read.
            let recorded = self
                .in_place_checkout()
                .ok()
                .and_then(|verified| super::in_place_changes::live(&verified, &self.records).ok())
                .unwrap_or(super::in_place_changes::LiveChanges {
                    changes: Vec::new(),
                    overflowed: false,
                });
            Some(in_place_facts(
                &InPlaceResult {
                    root_path: baseline.root_path(),
                    root_id: baseline.repository_id().as_str(),
                    snapshot_id: baseline.snapshot_id().as_str(),
                    root_git: record.git(),
                    recorded_files: recorded.changes.len() as u64,
                    recorded_bytes: recorded
                        .changes
                        .iter()
                        .fold(0u64, |total, change| total.saturating_add(change.bytes)),
                    recorded_overflowed: recorded.overflowed,
                    changed_paths: &paths,
                    paths_truncated: changed_paths_truncated.as_bool().unwrap_or(false),
                    finalized: finalized.is_some(),
                    finalize_source: finalize_source.as_deref(),
                    checkpoints,
                },
                self.job_id(),
            ))
        } else {
            None
        };
        let mut outcome = json!({
            "state": state,
            "answer": answer,
            "answer_truncated": answer_truncated,
            "commits": commits,
            "commits_truncated": commits_truncated,
            "commits_error": commits_error,
            "changed_paths": changed_paths,
            "changed_paths_truncated": changed_paths_truncated,
            "checkout": checkout,
            "readiness": envelope,
            "reason": reason,
            "reason_detail": reason_detail,
            "model_prose": "untrusted",
        });
        if let Some(Value::Object(facts)) = in_place {
            let object = outcome.as_object_mut().ok_or(CodingError::InvalidRequest)?;
            for (key, value) in facts {
                object.insert(key, value);
            }
        }
        let mut bytes = serde_json::to_vec(&outcome).map_err(CodingError::owner)?;
        if bytes.len() > MAX_OUTCOME_BYTES {
            outcome["changed_paths"] = json!([]);
            outcome["changed_paths_truncated"] = json!(true);
            bytes = serde_json::to_vec(&outcome).map_err(CodingError::owner)?;
        }
        context::write_replace(&self.records.join(OUTCOME_FILE), &bytes, MAX_OUTCOME_BYTES)
    }
}

/// The facts one in-place result states. The shape is a pure function of them,
/// so the published keys are provable without a run.
pub(super) struct InPlaceResult<'a> {
    pub(super) root_path: &'a str,
    pub(super) root_id: &'a str,
    pub(super) snapshot_id: &'a str,
    pub(super) root_git: bool,
    pub(super) recorded_files: u64,
    pub(super) recorded_bytes: u64,
    pub(super) recorded_overflowed: bool,
    pub(super) changed_paths: &'a [String],
    pub(super) paths_truncated: bool,
    pub(super) finalized: bool,
    pub(super) finalize_source: Option<&'a str>,
    pub(super) checkpoints: usize,
}

pub(super) fn in_place_facts(result: &InPlaceResult<'_>, job_id: &JobId) -> Value {
    let mut facts = json!({
        "mode": "in_place",
        "root": {
            "path": result.root_path,
            "root_id": result.root_id,
            "git": result.root_git,
        },
        "baseline": {
            "snapshot_id": result.snapshot_id,
            "kind": "edit_time",
        },
        "recorded": {
            "files": result.recorded_files,
            "bytes": result.recorded_bytes,
            "overflowed": result.recorded_overflowed,
        },
        "changes": {
            "files": result.changed_paths.len(),
            "paths": result.changed_paths,
            "paths_truncated": result.paths_truncated,
        },
        "review": {
            "source_sha256": result.finalize_source,
            "finalized": result.finalized,
        },
        "undo": {
            "checkpoints": result.checkpoints,
            "baseline_restore": true,
        },
    });
    // A job that changed nothing needs no warning: a read-only in-place job
    // says `changes: {files: 0}` and stays quiet.
    if !result.changed_paths.is_empty() {
        facts["notice"] = json!(format!(
            "This job edited {} in place. There is no Git history to recover from. \
             `saya job rewind {job_id} --baseline` restores the recorded before-images; \
             `saya job rewind {job_id} --to <turn>` restores a turn checkpoint. \
             A concurrent edit is reported and left as it stands unless the rewind is forced. \
             File content only: shell-made changes are reported, never rewound; commands, \
             downloads and database effects are not undone.",
            result.root_path
        ));
    }
    facts
}

/// The persisted outcome block, or `None` when the job has not finalized yet.
pub(super) fn read(records: &std::path::Path) -> Option<Value> {
    let path = records.join(OUTCOME_FILE);
    if !path.exists() {
        return None;
    }
    let bytes = context::read(&path, MAX_OUTCOME_BYTES).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// `text` bounded to `maximum` bytes at a character boundary, with a marker
/// past the limit; the bool says whether it was cut.
fn bounded(text: &str, maximum: usize) -> (String, bool) {
    if text.len() <= maximum {
        return (text.to_owned(), false);
    }
    let mut end = maximum;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (
        format!("{}\n[truncated at {maximum} bytes]", &text[..end]),
        true,
    )
}

#[cfg(all(test, unix))]
#[path = "outcome_tests.rs"]
mod tests;
