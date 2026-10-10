use serde::{Deserialize, Serialize};

use super::receipts::{ReceiptError, check_version};
use super::{JobId, MAX_RECEIPT_REFERENCE_BYTES, Sha256Digest};

pub const CHECK_RECEIPT_VERSION: u32 = 1;
pub const MAX_CHECK_RECEIPT_BYTES: usize = 64 * 1024;

/// Check execution status. Unknown and uncertain remain distinct from exit 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum CodingCheckOutcome {
    KnownExit { code: i32 },
    Unknown,
    Uncertain,
}

/// Evidence that a bounded check ran against a specific source digest.
/// Command identity is a safe reference plus the exact argv digest; raw argv
/// is omitted because it may contain credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckReceiptV1 {
    version: u32,
    job_id: JobId,
    source_sha256: Sha256Digest,
    command_id: String,
    argv_sha256: Sha256Digest,
    toolchain: String,
    environment_profile: String,
    outcome: CodingCheckOutcome,
}

impl CheckReceiptV1 {
    pub fn new(
        job_id: JobId,
        source_sha256: Sha256Digest,
        command_id: String,
        argv_sha256: Sha256Digest,
        toolchain: String,
        environment_profile: String,
        outcome: CodingCheckOutcome,
    ) -> Result<Self, ReceiptError> {
        for reference in [&command_id, &toolchain, &environment_profile] {
            validate_reference(reference)?;
        }
        let receipt = Self {
            version: CHECK_RECEIPT_VERSION,
            job_id,
            source_sha256,
            command_id,
            argv_sha256,
            toolchain,
            environment_profile,
            outcome,
        };
        let size = serde_json::to_vec(&receipt)
            .map_err(|_| ReceiptError::Malformed)?
            .len();
        if size > MAX_CHECK_RECEIPT_BYTES {
            return Err(ReceiptError::Oversize(size));
        }
        Ok(receipt)
    }

    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, ReceiptError> {
        check_version(bytes, MAX_CHECK_RECEIPT_BYTES, CHECK_RECEIPT_VERSION)?;
        let raw: WireCheckReceipt =
            serde_json::from_slice(bytes).map_err(|_| ReceiptError::Malformed)?;
        if raw.version != CHECK_RECEIPT_VERSION {
            return Err(ReceiptError::UnsupportedVersion(raw.version));
        }
        Self::new(
            raw.job_id,
            Sha256Digest::parse(&raw.source_sha256).map_err(|_| ReceiptError::InvalidDigest)?,
            raw.command_id,
            Sha256Digest::parse(&raw.argv_sha256).map_err(|_| ReceiptError::InvalidDigest)?,
            raw.toolchain,
            raw.environment_profile,
            raw.outcome,
        )
    }

    pub fn to_json_bytes(&self) -> Result<Vec<u8>, ReceiptError> {
        serde_json::to_vec(self).map_err(|_| ReceiptError::Malformed)
    }

    pub fn outcome(&self) -> CodingCheckOutcome {
        self.outcome
    }

    pub fn job_id(&self) -> &JobId {
        &self.job_id
    }

    pub fn source_sha256(&self) -> &Sha256Digest {
        &self.source_sha256
    }

    pub fn command_id(&self) -> &str {
        &self.command_id
    }

    pub fn argv_sha256(&self) -> &Sha256Digest {
        &self.argv_sha256
    }

    pub fn toolchain(&self) -> &str {
        &self.toolchain
    }

    pub fn environment_profile(&self) -> &str {
        &self.environment_profile
    }
}

fn validate_reference(value: &str) -> Result<(), ReceiptError> {
    if value.is_empty()
        || value.len() > MAX_RECEIPT_REFERENCE_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Err(ReceiptError::InvalidReference);
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireCheckReceipt {
    version: u32,
    job_id: JobId,
    source_sha256: String,
    command_id: String,
    argv_sha256: String,
    toolchain: String,
    environment_profile: String,
    outcome: CodingCheckOutcome,
}
