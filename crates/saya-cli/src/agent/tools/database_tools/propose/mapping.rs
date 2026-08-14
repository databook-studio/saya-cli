//! Mapping `ProposeOutcome` to the model-facing JSON result, and the wall-clock
//! stamp for evidence. The result carries the claim id, the outcome action, and
//! the status — never the opaque profile identity (the model sees the profile
//! *name* only, via the connection it named). A duplicate reports the existing
//! id and its real status, so a duplicate of a forgotten claim reads as
//! forgotten rather than as success (spec 3c §2).

use saya_store::ProposeOutcome;
use saya_types::ClaimStatus;

/// The model-facing result of a proposal.
pub(super) fn outcome_payload(outcome: ProposeOutcome) -> serde_json::Value {
    match outcome {
        ProposeOutcome::Stored(id) => serde_json::json!({
            "claim_id": id.as_str(),
            "action": "proposed",
            "status": ClaimStatus::Candidate.as_str(),
        }),
        ProposeOutcome::Duplicate { id, status } => serde_json::json!({
            "claim_id": id.as_str(),
            "action": "duplicate",
            "status": status.as_str(),
        }),
    }
}

/// Current wall-clock time in milliseconds since the Unix epoch, or 0 if the
/// clock is before the epoch (which would not be evidence worth recording).
pub(super) fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}
