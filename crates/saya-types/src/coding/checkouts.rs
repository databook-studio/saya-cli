use std::{fmt, num::NonZeroU64};

use serde::{
    Deserialize, Serialize,
    de::{Deserializer, Visitor},
};
use thiserror::Error;

use super::checkout_values::{GitObjectId, ManagedCheckoutLifecycle, ManagedCheckoutPath};
use super::control_wire::{self, ControlWireError};
use super::{AgentId, JobId, ManagedCheckoutId, RepositoryId, Sha256Digest, SnapshotId};

pub const MANAGED_CHECKOUT_RECORD_VERSION: u32 = 1;
pub const MAX_CHECKOUT_RECORD_BYTES: usize = control_wire::MAX_CONTROL_RECORD_BYTES;

/// Constructor input for a validated managed-checkout ownership record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedCheckoutRecordData {
    pub job_id: JobId,
    pub agent_id: Option<AgentId>,
    pub managed_checkout_id: ManagedCheckoutId,
    pub repository_id: RepositoryId,
    pub common_dir_sha256: Sha256Digest,
    pub managed_path: ManagedCheckoutPath,
    pub base_commit: GitObjectId,
    pub snapshot_id: SnapshotId,
    pub snapshot_sha256: Sha256Digest,
    pub created_by_sequence: NonZeroU64,
    pub lifecycle: ManagedCheckoutLifecycle,
}

/// A durable checkout ownership description, not proof of filesystem
/// ownership or permission to perform Git operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManagedCheckoutRecordV1 {
    version: u32,
    job_id: JobId,
    agent_id: Option<AgentId>,
    managed_checkout_id: ManagedCheckoutId,
    repository_id: RepositoryId,
    common_dir_sha256: Sha256Digest,
    managed_path: ManagedCheckoutPath,
    base_commit: GitObjectId,
    snapshot_id: SnapshotId,
    snapshot_sha256: Sha256Digest,
    created_by_sequence: NonZeroU64,
    lifecycle: ManagedCheckoutLifecycle,
}

impl ManagedCheckoutRecordV1 {
    pub fn new(data: ManagedCheckoutRecordData) -> Result<Self, CheckoutDtoError> {
        let record = Self {
            version: MANAGED_CHECKOUT_RECORD_VERSION,
            job_id: data.job_id,
            agent_id: data.agent_id,
            managed_checkout_id: data.managed_checkout_id,
            repository_id: data.repository_id,
            common_dir_sha256: data.common_dir_sha256,
            managed_path: data.managed_path,
            base_commit: data.base_commit,
            snapshot_id: data.snapshot_id,
            snapshot_sha256: data.snapshot_sha256,
            created_by_sequence: data.created_by_sequence,
            lifecycle: data.lifecycle,
        };
        control_wire::encode(&record).map_err(map_wire_error)?;
        Ok(record)
    }

    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, CheckoutDtoError> {
        control_wire::decode_versioned(bytes, MANAGED_CHECKOUT_RECORD_VERSION)
            .map_err(map_wire_error)
    }

    pub fn to_json_bytes(&self) -> Result<Vec<u8>, CheckoutDtoError> {
        control_wire::encode(self).map_err(map_wire_error)
    }

    pub fn job_id(&self) -> &JobId {
        &self.job_id
    }

    pub fn agent_id(&self) -> Option<&AgentId> {
        self.agent_id.as_ref()
    }

    pub fn managed_checkout_id(&self) -> &ManagedCheckoutId {
        &self.managed_checkout_id
    }

    pub fn repository_id(&self) -> &RepositoryId {
        &self.repository_id
    }

    pub fn common_dir_sha256(&self) -> &Sha256Digest {
        &self.common_dir_sha256
    }

    pub fn managed_path(&self) -> &ManagedCheckoutPath {
        &self.managed_path
    }

    pub fn base_commit(&self) -> &GitObjectId {
        &self.base_commit
    }

    pub fn snapshot_id(&self) -> &SnapshotId {
        &self.snapshot_id
    }

    pub fn snapshot_sha256(&self) -> &Sha256Digest {
        &self.snapshot_sha256
    }

    pub fn created_by_sequence(&self) -> NonZeroU64 {
        self.created_by_sequence
    }

    pub fn lifecycle(&self) -> ManagedCheckoutLifecycle {
        self.lifecycle
    }

    fn from_wire(wire: WireManagedCheckoutRecord) -> Result<Self, CheckoutDtoError> {
        if wire.version != MANAGED_CHECKOUT_RECORD_VERSION {
            return Err(CheckoutDtoError::UnsupportedVersion(wire.version));
        }
        Self::new(ManagedCheckoutRecordData {
            job_id: wire.job_id,
            agent_id: wire.agent_id.0,
            managed_checkout_id: wire.managed_checkout_id,
            repository_id: wire.repository_id,
            common_dir_sha256: wire.common_dir_sha256,
            managed_path: wire.managed_path,
            base_commit: wire.base_commit,
            snapshot_id: wire.snapshot_id,
            snapshot_sha256: wire.snapshot_sha256,
            created_by_sequence: wire.created_by_sequence,
            lifecycle: wire.lifecycle,
        })
    }
}

impl<'de> Deserialize<'de> for ManagedCheckoutRecordV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::from_wire(WireManagedCheckoutRecord::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
}

struct RequiredAgentId(Option<AgentId>);

impl<'de> Deserialize<'de> for RequiredAgentId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct RequiredAgentIdVisitor;

        impl<'de> Visitor<'de> for RequiredAgentIdVisitor {
            type Value = RequiredAgentId;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an agent ID string or null")
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(RequiredAgentId(None))
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(RequiredAgentId(None))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                AgentId::parse(value)
                    .map(Some)
                    .map(RequiredAgentId)
                    .map_err(E::custom)
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                AgentId::try_from(value)
                    .map(Some)
                    .map(RequiredAgentId)
                    .map_err(E::custom)
            }
        }

        deserializer.deserialize_any(RequiredAgentIdVisitor)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireManagedCheckoutRecord {
    version: u32,
    job_id: JobId,
    agent_id: RequiredAgentId,
    managed_checkout_id: ManagedCheckoutId,
    repository_id: RepositoryId,
    common_dir_sha256: Sha256Digest,
    managed_path: ManagedCheckoutPath,
    base_commit: GitObjectId,
    snapshot_id: SnapshotId,
    snapshot_sha256: Sha256Digest,
    created_by_sequence: NonZeroU64,
    lifecycle: ManagedCheckoutLifecycle,
}

pub(super) fn map_wire_error(error: ControlWireError) -> CheckoutDtoError {
    match error {
        ControlWireError::Oversize(bytes) => CheckoutDtoError::Oversize(bytes),
        ControlWireError::UnsupportedVersion(version) => {
            CheckoutDtoError::UnsupportedVersion(version)
        }
        ControlWireError::Malformed => CheckoutDtoError::Malformed,
    }
}

/// Why a versioned checkout record or operation was rejected.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum CheckoutDtoError {
    #[error("checkout record is {0} bytes, over the 65536-byte limit")]
    Oversize(usize),
    #[error("unsupported coding checkout version {0}")]
    UnsupportedVersion(u32),
    #[error("coding checkout record is malformed")]
    Malformed,
}
