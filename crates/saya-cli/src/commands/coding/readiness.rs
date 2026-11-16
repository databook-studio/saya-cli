//! Bounded, receipt-backed readiness for a write candidate.
use super::{NativeCodingJob, readiness_direct, readiness_steps};
use serde_json::{Value, json};

pub(super) struct CandidateReadiness(Value);

impl CandidateReadiness {
    pub(super) fn new(view: Value) -> Self {
        Self(view)
    }

    pub(super) fn view(&self) -> &Value {
        &self.0
    }

    pub(super) fn into_view(self) -> Value {
        self.0
    }
}

/// The final envelope a write-path run ends with: the shared keys plus the
/// facts the readiness view carries. A write-path job ends on the advisory view
/// (W, the live children, the suggestion); a job with no write path has no
/// candidate at all, so only its `next_action` is carried.
pub(super) fn final_envelope(readiness: &CandidateReadiness) -> Value {
    let value = readiness.view();
    let facts: &[&str] = if value.get("suggestion").is_some() {
        &[
            "w_changed",
            "live_children",
            "live_children_omitted",
            "suggestion",
        ]
    } else {
        &["next_action"]
    };
    let mut envelope = json!({
        "candidate_ready": value["candidate_ready"],
        "candidate": value["candidate"],
        "source_generation_sha256": value["source_generation_sha256"],
        "apply_requires_fresh_approval": value["state"] == "candidate_ready",
        "model_prose": "untrusted",
        "readiness": value
    });
    for key in facts {
        envelope[*key] = value[*key].clone();
    }
    envelope
}

impl NativeCodingJob {
    /// A job with no approved write path has no candidate to prepare, so its
    /// readiness is the shared not-applicable view; every write-path job's is
    /// the advisory direct view.
    pub(super) async fn candidate_readiness(&self) -> CandidateReadiness {
        if self.request.write_paths.is_empty() {
            return readiness_steps::not_applicable();
        }
        readiness_direct::direct(self).await
    }
}
