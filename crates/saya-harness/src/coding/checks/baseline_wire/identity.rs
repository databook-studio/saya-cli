//! Baseline identity: the receipt-derived key, its digest, and receipt comparison.

use saya_types::{CodingCheckOutcome, JobId, ManagedCheckoutId, Sha256Digest};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::super::super::{CheckDiagnostics, CheckReceipt};
use super::DiagnosticBaselineWireError;

/// Immutable identity needed to compare a later receipt with the baseline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BaselineIdentity {
    pub(crate) job_id: JobId,
    pub(crate) checkout_id: ManagedCheckoutId,
    pub(crate) profile_id: String,
    pub(crate) source_sha256: Sha256Digest,
    pub(crate) comparison_sha256: Sha256Digest,
    pub(crate) outcome: CodingCheckOutcome,
    pub(crate) output_complete: bool,
}

impl BaselineIdentity {
    pub(super) fn from_receipt(evidence: &CheckDiagnostics) -> Self {
        Self::from_parts(evidence.receipt())
    }

    pub(crate) fn from_parts(receipt: &CheckReceipt) -> Self {
        let wire = receipt.wire_receipt();
        Self {
            job_id: wire.job_id().clone(),
            checkout_id: receipt.checkout_id().clone(),
            profile_id: receipt.profile_id().to_owned(),
            source_sha256: wire.source_sha256().clone(),
            comparison_sha256: receipt_identity_digest(receipt),
            outcome: receipt.outcome(),
            output_complete: receipt.output().complete(),
        }
    }
}

impl BaselineIdentity {
    pub(super) fn validate(self) -> Result<Self, DiagnosticBaselineWireError> {
        if !valid_reference(&self.profile_id)
            || !self.output_complete
            || !matches!(self.outcome, CodingCheckOutcome::KnownExit { .. })
        {
            return Err(DiagnosticBaselineWireError::IdentityMismatch);
        }
        Ok(self)
    }
}

fn receipt_identity_digest(receipt: &CheckReceipt) -> Sha256Digest {
    let bytes = serde_json::to_vec(&(
        receipt.wire.job_id(),
        receipt.checkout_id.as_str(),
        receipt.profile_id.as_str(),
        receipt.program.as_str(),
        receipt.toolchain.as_deref(),
        receipt.wire.command_id(),
        receipt.wire.argv_sha256(),
        receipt.wire.environment_profile(),
        receipt.lineage.checkout.as_str(),
        (
            receipt.lineage.path.as_str(),
            receipt.lineage.repository.as_str(),
            receipt.lineage.snapshot.as_str(),
            receipt.lineage.snapshot_sha256.as_str(),
            receipt.lineage.plan_sha256.as_str(),
            receipt.lineage.base.as_str(),
            receipt.lineage.agent.as_ref().map(|agent| agent.as_str()),
            receipt.lineage.policy.as_str(),
            receipt.lineage.epoch,
        ),
    ))
    .expect("receipt identity serializes");
    let digest = Sha256::digest(bytes);
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    Sha256Digest::parse(&hex).expect("sha256 encoding is valid")
}

pub(crate) fn receipt_matches(receipt: &CheckReceipt, baseline: &BaselineIdentity) -> bool {
    receipt.checkout_id == baseline.checkout_id
        && receipt.profile_id == baseline.profile_id
        && receipt.wire.job_id() == &baseline.job_id
        && receipt_identity_digest(receipt) == baseline.comparison_sha256
}

fn valid_reference(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}
