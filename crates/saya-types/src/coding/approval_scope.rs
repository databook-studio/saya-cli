use std::{net::IpAddr, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{ManagedCheckoutId, RepositoryId, Sha256Digest};
use crate::ProfileIdentity;

const MAX_SCOPE_PATH_BYTES: usize = 512;
const MAX_COMMAND_REFERENCE_BYTES: usize = 128;
const MAX_MEMORY_KEY_BYTES: usize = 128;
const MAX_NETWORK_HOST_BYTES: usize = 253;

/// The finite network schemes currently supported by coding approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ApprovalNetworkScheme {
    Http,
    Https,
}

/// A credential-free host-only network destination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApprovalNetworkDestination {
    scheme: ApprovalNetworkScheme,
    host: String,
}

impl ApprovalNetworkDestination {
    pub fn new(
        scheme: ApprovalNetworkScheme,
        host: impl Into<String>,
    ) -> Result<Self, ApprovalScopeError> {
        let host = host.into();
        if host.is_empty() || host.len() > MAX_NETWORK_HOST_BYTES || !host.is_ascii() {
            return Err(ApprovalScopeError::InvalidDestination);
        }
        let host = match IpAddr::from_str(&host) {
            Ok(address) => address.to_string(),
            Err(_) => validate_dns_host(&host)?,
        };
        Ok(Self { scheme, host })
    }

    pub fn scheme(&self) -> ApprovalNetworkScheme {
        self.scheme
    }

    pub fn host(&self) -> &str {
        &self.host
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireNetworkDestination {
    scheme: ApprovalNetworkScheme,
    host: String,
}

impl<'de> Deserialize<'de> for ApprovalNetworkDestination {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = WireNetworkDestination::deserialize(deserializer)?;
        Self::new(wire.scheme, wire.host).map_err(serde::de::Error::custom)
    }
}

/// One exact pending operation and its bounded resource. This shape carries
/// consent context only; a runtime must still check live root authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum ApprovalScope {
    WorkspaceWrite {
        managed_checkout_id: ManagedCheckoutId,
        relative_path: WorkspacePath,
    },
    ProcessExecution {
        managed_checkout_id: ManagedCheckoutId,
        command_reference: SafeCommandReference,
        argv_sha256: Sha256Digest,
        cwd: WorkingDirectory,
    },
    NetworkAccess {
        destination: ApprovalNetworkDestination,
    },
    DatabaseRead {
        profile_identity: ProfileIdentity,
        statement_sha256: Sha256Digest,
    },
    MemoryWrite {
        repository_id: RepositoryId,
        memory_key: MemoryKey,
    },
    RepositoryMutation {
        managed_checkout_id: ManagedCheckoutId,
        action: RepositoryMutationAction,
    },
}

impl ApprovalScope {
    pub fn workspace_write(
        managed_checkout_id: ManagedCheckoutId,
        relative_path: impl Into<String>,
    ) -> Result<Self, ApprovalScopeError> {
        Ok(Self::WorkspaceWrite {
            managed_checkout_id,
            relative_path: WorkspacePath::new(relative_path.into())?,
        })
    }

    pub fn process_execution(
        managed_checkout_id: ManagedCheckoutId,
        command_reference: impl Into<String>,
        argv_sha256: Sha256Digest,
        cwd: impl Into<String>,
    ) -> Result<Self, ApprovalScopeError> {
        Ok(Self::ProcessExecution {
            managed_checkout_id,
            command_reference: SafeCommandReference::new(command_reference.into())?,
            argv_sha256,
            cwd: WorkingDirectory::new(cwd.into())?,
        })
    }

    pub fn network_access(destination: ApprovalNetworkDestination) -> Self {
        Self::NetworkAccess { destination }
    }

    pub fn database_read(
        profile_identity: ProfileIdentity,
        statement_sha256: Sha256Digest,
    ) -> Self {
        Self::DatabaseRead {
            profile_identity,
            statement_sha256,
        }
    }

    pub fn memory_write(
        repository_id: RepositoryId,
        memory_key: impl Into<String>,
    ) -> Result<Self, ApprovalScopeError> {
        Ok(Self::MemoryWrite {
            repository_id,
            memory_key: MemoryKey::new(memory_key.into())?,
        })
    }

    pub fn repository_mutation(
        managed_checkout_id: ManagedCheckoutId,
        action: RepositoryMutationAction,
    ) -> Self {
        Self::RepositoryMutation {
            managed_checkout_id,
            action,
        }
    }
}

/// A finite mutation request; it never authorizes Git by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RepositoryMutationAction {
    CreateCheckout,
    RemoveCheckout,
    IntegrateCandidate,
    ApplyBack,
}

/// A validated workspace path held inside a write request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkspacePath(String);

impl WorkspacePath {
    fn new(value: String) -> Result<Self, ApprovalScopeError> {
        validate_relative_path(&value, false)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for WorkspacePath {
    type Error = ApprovalScopeError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for WorkspacePath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .try_into()
            .map_err(serde::de::Error::custom)
    }
}

/// A validated checkout-relative process working directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkingDirectory(String);

impl WorkingDirectory {
    fn new(value: String) -> Result<Self, ApprovalScopeError> {
        validate_relative_path(&value, true)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for WorkingDirectory {
    type Error = ApprovalScopeError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for WorkingDirectory {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .try_into()
            .map_err(serde::de::Error::custom)
    }
}

/// A safe reference naming a configured command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SafeCommandReference(String);

impl SafeCommandReference {
    fn new(value: String) -> Result<Self, ApprovalScopeError> {
        if value.is_empty()
            || value.len() > MAX_COMMAND_REFERENCE_BYTES
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        {
            return Err(ApprovalScopeError::InvalidCommandReference);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SafeCommandReference {
    type Error = ApprovalScopeError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for SafeCommandReference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .try_into()
            .map_err(serde::de::Error::custom)
    }
}

/// A bounded key naming a repository memory entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemoryKey(String);

impl MemoryKey {
    fn new(value: String) -> Result<Self, ApprovalScopeError> {
        if value.is_empty()
            || value.len() > MAX_MEMORY_KEY_BYTES
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(ApprovalScopeError::InvalidMemoryKey);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for MemoryKey {
    type Error = ApprovalScopeError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for MemoryKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .try_into()
            .map_err(serde::de::Error::custom)
    }
}

fn validate_relative_path(value: &str, allow_root: bool) -> Result<(), ApprovalScopeError> {
    if allow_root && value == "." {
        return Ok(());
    }
    if value.is_empty()
        || value.len() > MAX_SCOPE_PATH_BYTES
        || value.starts_with('/')
        || value.as_bytes().get(1) == Some(&b':') && value.as_bytes()[0].is_ascii_alphabetic()
        || value.contains('\\')
        || value.chars().any(char::is_control)
        || value
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(ApprovalScopeError::InvalidPath);
    }
    Ok(())
}

fn validate_dns_host(host: &str) -> Result<String, ApprovalScopeError> {
    let host = host.to_ascii_lowercase();
    if host.len() > MAX_NETWORK_HOST_BYTES
        || host.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        return Err(ApprovalScopeError::InvalidDestination);
    }
    Ok(host)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ApprovalScopeError {
    #[error("approval scope contains an invalid path")]
    InvalidPath,
    #[error("approval command reference is not a safe reference")]
    InvalidCommandReference,
    #[error("approval memory key is not a safe key")]
    InvalidMemoryKey,
    #[error("approval network destination is not a supported host-only destination")]
    InvalidDestination,
}
