//! Strict, content-free persistent evidence for one prepared candidate.
use super::{CodingError, CodingReview};
use saya_types::{JobId, ManagedCheckoutId, Sha256Digest};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub(super) const MAX_PREPARED_RECEIPT_BYTES: usize = 64 * 1024;
const PREPARED_RECEIPT_VERSION: u32 = 1;

/// Check execution status, mirroring the `saya-types` wire type field for
/// field so `prepared-candidate-*.json` files that still carry checks parse.
/// Nothing constructs a non-`Unknown` variant; the derived `Deserialize` is
/// the only constructor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum PreparedCheckOutcome {
    KnownExit { code: i32 },
    Unknown,
    Uncertain,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PreparedReceipt {
    version: u32,
    source_generation_sha256: Sha256Digest,
    candidate: ManagedCheckoutId,
    reviewed_source_sha256: Sha256Digest,
    /// Check receipts a prepared candidate may carry. Nothing populates it,
    /// but the field stays — with a serde default — so
    /// `prepared-candidate-*.json` files that still carry checks parse.
    #[serde(default)]
    checks: Vec<PreparedCheck>,
    candidate_ready: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedCheck {
    profile: String,
    receipt: PreparedCheckReceipt,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PreparedCheckReceipt {
    pub(super) version: u32,
    pub(super) job_id: JobId,
    pub(super) source_sha256: Sha256Digest,
    pub(super) command_id: String,
    pub(super) argv_sha256: Sha256Digest,
    pub(super) toolchain: String,
    pub(super) environment_profile: String,
    #[serde(default)]
    pub(super) managed_snapshot_sha256: Option<Sha256Digest>,
    #[serde(default)]
    pub(super) managed_input_key_sha256: Option<Sha256Digest>,
    #[serde(default)]
    pub(super) managed_bundle_sha256: Option<Sha256Digest>,
    pub(super) outcome: PreparedCheckOutcome,
}

impl PreparedReceipt {
    pub(super) fn from_review(
        generation: Sha256Digest,
        candidate: ManagedCheckoutId,
        review: &CodingReview,
    ) -> Result<Self, CodingError> {
        Ok(Self {
            version: PREPARED_RECEIPT_VERSION,
            source_generation_sha256: generation,
            candidate,
            reviewed_source_sha256: review.source_sha256().clone(),
            // No check runs, so a prepared candidate is ready: the
            // operator's review and apply is the check.
            candidate_ready: true,
            checks: Vec::new(),
        })
    }

    pub(super) fn from_bytes(bytes: &[u8]) -> Result<Self, CodingError> {
        if bytes.len() > MAX_PREPARED_RECEIPT_BYTES {
            return Err(CodingError::PreparedCandidateState);
        }
        serde_json::from_slice(bytes).map_err(|_| CodingError::PreparedCandidateState)
    }

    pub(super) fn generation(&self) -> &Sha256Digest {
        &self.source_generation_sha256
    }

    pub(super) fn candidate(&self) -> &ManagedCheckoutId {
        &self.candidate
    }

    pub(super) fn reviewed_source_sha256(&self) -> &Sha256Digest {
        &self.reviewed_source_sha256
    }

    pub(super) fn validate_shape(
        &self,
        expected_job: &JobId,
        expected_source: &Sha256Digest,
    ) -> Result<(), CodingError> {
        // A receipt may still carry checks:
        // its own `candidate_ready` must agree with them, and a receipt with no
        // check at all is ready by construction.
        if self.version != PREPARED_RECEIPT_VERSION
            || self.checks.len() > 64
            || &self.reviewed_source_sha256 != expected_source
            || self.candidate_ready != self.checks.iter().all(PreparedCheck::successful)
        {
            return Err(CodingError::PreparedCandidateState);
        }
        for (profile, check) in self.checks() {
            if !valid_reference(profile)
                || check.job_id != *expected_job
                || check.source_sha256 != *expected_source
            {
                return Err(CodingError::PreparedCandidateState);
            }
        }
        Ok(())
    }

    pub(super) fn checks(&self) -> impl Iterator<Item = (&str, &PreparedCheckReceipt)> {
        self.checks
            .iter()
            .map(|check| (check.profile.as_str(), &check.receipt))
    }

    pub(super) fn candidate_ready(&self) -> bool {
        self.candidate_ready
    }

    pub(super) fn into_value(self) -> Value {
        json!({
            "version": self.version,
            "source_generation_sha256": self.source_generation_sha256,
            "candidate": self.candidate,
            "reviewed_source_sha256": self.reviewed_source_sha256,
            "checks": self.checks.iter().map(PreparedCheck::view).collect::<Vec<_>>(),
            "candidate_ready": self.candidate_ready,
        })
    }
}

impl PreparedCheck {
    fn successful(&self) -> bool {
        matches!(
            self.receipt.outcome,
            PreparedCheckOutcome::KnownExit { code: 0 }
        )
    }

    fn view(&self) -> Value {
        json!({"profile": self.profile, "receipt": self.receipt, "successful": self.successful()})
    }
}

fn valid_reference(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}
