use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{AgentId, InvocationId, JobId, Sha256Digest};

pub const RESULT_RECEIPT_VERSION: u32 = 1;
pub const MAX_RESULT_RECEIPT_BYTES: usize = 64 * 1024;
pub const MAX_RECEIPT_PATHS: usize = 128;
pub const MAX_RECEIPT_PATH_BYTES: usize = 512;
pub const MAX_RECEIPT_PATH_SET_BYTES: usize = 32 * 1024;
pub const MAX_RECEIPT_REFERENCE_BYTES: usize = 128;

/// The reported outcome of a child result. A receipt records a claim, not a
/// correctness verdict or authority grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CodingResultOutcome {
    Completed,
    Failed,
    Cancelled,
    Uncertain,
}

/// Source-bound result metadata. Paths are a unique relative set; their
/// ownership and contents are verified by the harness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResultReceiptV1 {
    version: u32,
    job_id: JobId,
    agent_id: AgentId,
    invocation_id: InvocationId,
    source_sha256: Sha256Digest,
    paths: Vec<String>,
    outcome: CodingResultOutcome,
}

impl ResultReceiptV1 {
    pub fn new(
        job_id: JobId,
        agent_id: AgentId,
        invocation_id: InvocationId,
        source_sha256: Sha256Digest,
        paths: Vec<String>,
        outcome: CodingResultOutcome,
    ) -> Result<Self, ReceiptError> {
        validate_paths(&paths)?;
        let receipt = Self {
            version: RESULT_RECEIPT_VERSION,
            job_id,
            agent_id,
            invocation_id,
            source_sha256,
            paths,
            outcome,
        };
        receipt.ensure_size(MAX_RESULT_RECEIPT_BYTES)?;
        Ok(receipt)
    }

    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, ReceiptError> {
        check_version(bytes, MAX_RESULT_RECEIPT_BYTES, RESULT_RECEIPT_VERSION)?;
        let raw: WireResultReceipt =
            serde_json::from_slice(bytes).map_err(|_| ReceiptError::Malformed)?;
        if raw.version != RESULT_RECEIPT_VERSION {
            return Err(ReceiptError::UnsupportedVersion(raw.version));
        }
        let source_sha256 =
            Sha256Digest::parse(&raw.source_sha256).map_err(|_| ReceiptError::InvalidDigest)?;
        Self::new(
            raw.job_id,
            raw.agent_id,
            raw.invocation_id,
            source_sha256,
            raw.paths,
            raw.outcome,
        )
    }

    pub fn to_json_bytes(&self) -> Result<Vec<u8>, ReceiptError> {
        let bytes = serde_json::to_vec(self).map_err(|_| ReceiptError::Malformed)?;
        if bytes.len() > MAX_RESULT_RECEIPT_BYTES {
            return Err(ReceiptError::Oversize(bytes.len()));
        }
        Ok(bytes)
    }

    pub fn source_sha256(&self) -> &Sha256Digest {
        &self.source_sha256
    }

    pub fn job_id(&self) -> &JobId {
        &self.job_id
    }

    pub fn agent_id(&self) -> &AgentId {
        &self.agent_id
    }

    pub fn invocation_id(&self) -> &InvocationId {
        &self.invocation_id
    }

    pub fn paths(&self) -> &[String] {
        &self.paths
    }

    pub fn outcome(&self) -> CodingResultOutcome {
        self.outcome
    }

    fn ensure_size(&self, limit: usize) -> Result<(), ReceiptError> {
        let size = serde_json::to_vec(self)
            .map_err(|_| ReceiptError::Malformed)?
            .len();
        if size > limit {
            return Err(ReceiptError::Oversize(size));
        }
        Ok(())
    }
}

pub(super) fn check_version(
    bytes: &[u8],
    maximum: usize,
    supported: u32,
) -> Result<(), ReceiptError> {
    if bytes.len() > maximum {
        return Err(ReceiptError::Oversize(bytes.len()));
    }
    let probe: VersionProbe = serde_json::from_slice(bytes).map_err(|_| ReceiptError::Malformed)?;
    match probe.version {
        Some(version) if version != supported => Err(ReceiptError::UnsupportedVersion(version)),
        Some(_) => Ok(()),
        None => Err(ReceiptError::Malformed),
    }
}

fn validate_paths(paths: &[String]) -> Result<(), ReceiptError> {
    if paths.len() > MAX_RECEIPT_PATHS {
        return Err(ReceiptError::TooManyPaths);
    }
    let mut seen = HashSet::with_capacity(paths.len());
    let mut total_bytes = 0usize;
    for path in paths {
        validate_path(path)?;
        total_bytes += path.len();
        if total_bytes > MAX_RECEIPT_PATH_SET_BYTES {
            return Err(ReceiptError::PathSetTooLarge);
        }
        if !seen.insert(path.as_str()) {
            return Err(ReceiptError::DuplicatePath);
        }
    }
    Ok(())
}

fn validate_path(path: &str) -> Result<(), ReceiptError> {
    if path.is_empty() || path.len() > MAX_RECEIPT_PATH_BYTES {
        return Err(ReceiptError::PathTooLong);
    }
    let bytes = path.as_bytes();
    let absolute = bytes.first().is_some_and(|b| matches!(b, b'/' | b'\\'))
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':');
    if path.chars().any(char::is_control) || absolute {
        return Err(ReceiptError::InvalidPath);
    }
    if path
        .split(['/', '\\'])
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(ReceiptError::InvalidPath);
    }
    Ok(())
}

#[derive(Deserialize)]
struct VersionProbe {
    version: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireResultReceipt {
    version: u32,
    job_id: JobId,
    agent_id: AgentId,
    invocation_id: InvocationId,
    source_sha256: String,
    paths: Vec<String>,
    outcome: CodingResultOutcome,
}

/// Why a result or check receipt could not be constructed or decoded.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ReceiptError {
    #[error("receipt is {0} bytes, over its serialized limit")]
    Oversize(usize),
    #[error("unsupported coding receipt version {0}")]
    UnsupportedVersion(u32),
    #[error("coding receipt is malformed")]
    Malformed,
    #[error("receipt source or argv digest is not valid lowercase SHA-256")]
    InvalidDigest,
    #[error("result receipt declares too many paths")]
    TooManyPaths,
    #[error("result receipt path exceeds {MAX_RECEIPT_PATH_BYTES} bytes")]
    PathTooLong,
    #[error("result receipt contains an absolute or escaping path")]
    InvalidPath,
    #[error("result receipt paths exceed {MAX_RECEIPT_PATH_SET_BYTES} aggregate bytes")]
    PathSetTooLarge,
    #[error("result receipt paths must be unique")]
    DuplicatePath,
    #[error("receipt reference must be 1-{MAX_RECEIPT_REFERENCE_BYTES} safe ASCII bytes")]
    InvalidReference,
}
