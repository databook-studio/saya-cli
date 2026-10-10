use std::num::NonZeroU64;

use serde::{Deserialize, Serialize, de::Deserializer};
use thiserror::Error;

use super::approval_scope::ApprovalScope;
use super::control_wire::{self, ControlWireError};
use super::{AgentId, InvocationId, JobId, Sha256Digest};

pub const APPROVAL_REQUEST_VERSION: u32 = 1;
pub const MAX_APPROVAL_REQUEST_BYTES: usize = super::control_wire::MAX_CONTROL_RECORD_BYTES;

/// Trusted policy and lifetime context captured with one pending request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequestContext {
    policy_sha256: Sha256Digest,
    revocation_epoch: u64,
    requested_at_unix_ms: u64,
    expires_at_unix_ms: u64,
}

impl ApprovalRequestContext {
    pub fn new(
        policy_sha256: Sha256Digest,
        revocation_epoch: u64,
        requested_at_unix_ms: u64,
        expires_at_unix_ms: u64,
    ) -> Result<Self, ApprovalRequestError> {
        if expires_at_unix_ms <= requested_at_unix_ms {
            return Err(ApprovalRequestError::InvalidExpiry);
        }
        Ok(Self {
            policy_sha256,
            revocation_epoch,
            requested_at_unix_ms,
            expires_at_unix_ms,
        })
    }
}

/// A finite pending consent request. It records no approval decision or grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApprovalRequestV1 {
    version: u32,
    job_id: JobId,
    agent_id: AgentId,
    invocation_id: InvocationId,
    request_sequence: NonZeroU64,
    policy_sha256: Sha256Digest,
    revocation_epoch: u64,
    requested_at_unix_ms: u64,
    expires_at_unix_ms: u64,
    scope: ApprovalScope,
}

impl ApprovalRequestV1 {
    pub fn new(
        job_id: JobId,
        agent_id: AgentId,
        invocation_id: InvocationId,
        request_sequence: NonZeroU64,
        context: ApprovalRequestContext,
        scope: ApprovalScope,
    ) -> Result<Self, ApprovalRequestError> {
        let request = Self {
            version: APPROVAL_REQUEST_VERSION,
            job_id,
            agent_id,
            invocation_id,
            request_sequence,
            policy_sha256: context.policy_sha256,
            revocation_epoch: context.revocation_epoch,
            requested_at_unix_ms: context.requested_at_unix_ms,
            expires_at_unix_ms: context.expires_at_unix_ms,
            scope,
        };
        control_wire::encode(&request).map_err(map_wire_error)?;
        Ok(request)
    }

    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, ApprovalRequestError> {
        control_wire::decode_versioned(bytes, APPROVAL_REQUEST_VERSION).map_err(map_wire_error)
    }

    pub fn to_json_bytes(&self) -> Result<Vec<u8>, ApprovalRequestError> {
        control_wire::encode(self).map_err(map_wire_error)
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

    pub fn request_sequence(&self) -> NonZeroU64 {
        self.request_sequence
    }

    pub fn policy_sha256(&self) -> &Sha256Digest {
        &self.policy_sha256
    }

    pub fn revocation_epoch(&self) -> u64 {
        self.revocation_epoch
    }

    pub fn requested_at_unix_ms(&self) -> u64 {
        self.requested_at_unix_ms
    }

    pub fn expires_at_unix_ms(&self) -> u64 {
        self.expires_at_unix_ms
    }

    pub fn scope(&self) -> &ApprovalScope {
        &self.scope
    }

    fn from_wire(wire: WireApprovalRequest) -> Result<Self, ApprovalRequestError> {
        if wire.version != APPROVAL_REQUEST_VERSION {
            return Err(ApprovalRequestError::UnsupportedVersion(wire.version));
        }
        let context = ApprovalRequestContext::new(
            wire.policy_sha256,
            wire.revocation_epoch,
            wire.requested_at_unix_ms,
            wire.expires_at_unix_ms,
        )?;
        Self::new(
            wire.job_id,
            wire.agent_id,
            wire.invocation_id,
            wire.request_sequence,
            context,
            wire.scope,
        )
    }
}

impl<'de> Deserialize<'de> for ApprovalRequestV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::from_wire(WireApprovalRequest::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
}

fn map_wire_error(error: ControlWireError) -> ApprovalRequestError {
    match error {
        ControlWireError::Oversize(bytes) => ApprovalRequestError::Oversize(bytes),
        ControlWireError::UnsupportedVersion(version) => {
            ApprovalRequestError::UnsupportedVersion(version)
        }
        ControlWireError::Malformed => ApprovalRequestError::Malformed,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireApprovalRequest {
    version: u32,
    job_id: JobId,
    agent_id: AgentId,
    invocation_id: InvocationId,
    request_sequence: NonZeroU64,
    policy_sha256: Sha256Digest,
    revocation_epoch: u64,
    requested_at_unix_ms: u64,
    expires_at_unix_ms: u64,
    scope: ApprovalScope,
}

/// Why a pending approval request was rejected.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ApprovalRequestError {
    #[error("approval request is {0} bytes, over the 65536-byte limit")]
    Oversize(usize),
    #[error("unsupported coding approval request version {0}")]
    UnsupportedVersion(u32),
    #[error("coding approval request is malformed")]
    Malformed,
    #[error("approval request expiry must be later than its request time")]
    InvalidExpiry,
}
