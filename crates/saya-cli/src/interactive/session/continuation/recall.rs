use crate::contracts::{RecallOutcomeKind, RecallReceipt};
use saya_types::ClaimStatus;

pub(super) fn supplied_counts(receipt: &RecallReceipt) -> (usize, usize) {
    let mut confirmed = 0;
    let mut candidate = 0;
    for claim in receipt.supplied.iter().flat_map(|entry| &entry.claims) {
        match claim.status {
            ClaimStatus::Confirmed => confirmed += 1,
            ClaimStatus::Candidate => candidate += 1,
            _ => {}
        }
    }
    (confirmed, candidate)
}

pub(super) fn label(receipt: &RecallReceipt) -> &'static str {
    match receipt.kind {
        RecallOutcomeKind::ConfiguredOff => "configured off",
        RecallOutcomeKind::PrivacyGateClosed => "privacy gate closed",
        RecallOutcomeKind::Ran {
            store_unavailable: true,
        } => "ran; store unavailable",
        RecallOutcomeKind::Ran {
            store_unavailable: false,
        } => "ran",
    }
}
