use std::fmt;

use saya_types::GitObjectId;
use serde::{Deserialize, Serialize};

use super::SnapshotError;

pub const MAX_SNAPSHOT_FILES: usize = 20_000;
pub const MAX_SNAPSHOT_FILE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_SNAPSHOT_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_SNAPSHOT_MANIFEST_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_PRIVATE_PREFIXES: usize = 128;
pub(crate) const MAX_PRIVATE_PREFIX_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotSelection {
    Head,
    CurrentWorkingContent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SnapshotContentSource {
    GitBlob { object_id: GitObjectId },
    WorkingTree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotFileMode {
    Regular,
    Executable,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct SnapshotPath(String);

impl SnapshotPath {
    pub(crate) fn parse(value: String) -> Result<Self, SnapshotError> {
        if value.is_empty()
            || value.len() > 512
            || value.starts_with('/')
            || value.contains('\\')
            || value.as_bytes().contains(&0)
            || value.split('/').any(|part| {
                part.is_empty() || part == "." || part == ".." || part.eq_ignore_ascii_case(".git")
            })
            || value.as_bytes().get(1) == Some(&b':')
        {
            return Err(SnapshotError::UnsafePath);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for SnapshotPath {
    /// Re-validates on read: a stored path is never trusted.
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(|_| serde::de::Error::custom("unsafe snapshot path"))
    }
}

impl fmt::Debug for SnapshotPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[snapshot path]")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotOmissionReason {
    Ignored,
    ConfiguredPrivatePath,
    SensitiveUntracked,
}

impl SnapshotOmissionReason {
    pub fn is_ignored(self) -> bool {
        self == Self::Ignored
    }
}

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct SnapshotOmissionFact {
    pub(super) path: SnapshotPath,
    pub(super) reason: SnapshotOmissionReason,
}

impl SnapshotOmissionFact {
    pub fn path(&self) -> &SnapshotPath {
        &self.path
    }

    pub fn reason(&self) -> SnapshotOmissionReason {
        self.reason
    }
}

impl fmt::Debug for SnapshotOmissionFact {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SnapshotOmissionFact")
            .field("path", &"[snapshot path]")
            .field("reason", &self.reason)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct SnapshotFileEntry {
    pub(super) path: SnapshotPath,
    pub(super) source: SnapshotContentSource,
    pub(super) mode: SnapshotFileMode,
    pub(super) declared_bytes: u64,
    pub(super) source_identity: Option<SnapshotSourceIdentity>,
}

impl SnapshotFileEntry {
    pub fn path(&self) -> &SnapshotPath {
        &self.path
    }

    pub fn source(&self) -> &SnapshotContentSource {
        &self.source
    }

    pub fn mode(&self) -> SnapshotFileMode {
        self.mode
    }

    pub fn declared_bytes(&self) -> u64 {
        self.declared_bytes
    }

    pub fn source_identity(&self) -> Option<&SnapshotSourceIdentity> {
        self.source_identity.as_ref()
    }
}

impl fmt::Debug for SnapshotFileEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SnapshotFileEntry")
            .field("path", &"[snapshot path]")
            .field("source", &self.source)
            .field("mode", &self.mode)
            .field("declared_bytes", &self.declared_bytes)
            .field("source_identity", &self.source_identity)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SnapshotSourceIdentity {
    pub(super) device: u64,
    pub(super) inode: u64,
    pub(super) size: u64,
    pub(super) mode: u32,
}

impl SnapshotSourceIdentity {
    pub(crate) fn new(device: u64, inode: u64, size: u64, mode: u32) -> Self {
        Self {
            device,
            inode,
            size,
            mode,
        }
    }

    pub fn size(&self) -> u64 {
        self.size
    }
}

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct SnapshotIndexEntry {
    pub(super) path: SnapshotPath,
    pub(super) mode: u32,
    pub(super) object_id: GitObjectId,
    pub(super) stage: u8,
    pub(super) intent_to_add: bool,
}

impl SnapshotIndexEntry {
    pub fn path(&self) -> &SnapshotPath {
        &self.path
    }

    pub fn mode(&self) -> u32 {
        self.mode
    }

    pub fn object_id(&self) -> &GitObjectId {
        &self.object_id
    }

    pub fn stage(&self) -> u8 {
        self.stage
    }

    pub fn intent_to_add(&self) -> bool {
        self.intent_to_add
    }
}

impl fmt::Debug for SnapshotIndexEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SnapshotIndexEntry")
            .field("path", &"[snapshot path]")
            .field("mode", &self.mode)
            .field("object_id", &self.object_id)
            .field("stage", &self.stage)
            .field("intent_to_add", &self.intent_to_add)
            .finish()
    }
}
