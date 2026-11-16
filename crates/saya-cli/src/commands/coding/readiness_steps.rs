//! The one readiness view a job with no candidate can have.
use super::readiness::CandidateReadiness;
use serde_json::json;

/// The shared not-applicable view: a job with no approved write paths has no
/// candidate to prepare.
pub(super) fn not_applicable() -> CandidateReadiness {
    CandidateReadiness::new(json!({
        "state": "not_applicable",
        "candidate_ready": false,
        "next_action": null
    }))
}
