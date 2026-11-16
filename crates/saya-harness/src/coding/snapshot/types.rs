use std::fmt;

use saya_types::GitObjectId;
use serde::{Deserialize, Serialize};

use super::SnapshotError;

/// The job snapshot's file-count bound. A real repository of ~20k tracked
/// files must fit with headroom for untracked work in progress, so the bound
/// is 50k files rather than the 20k a medium repository already reaches.
pub const MAX_SNAPSHOT_FILES: usize = 50_000;
pub const MAX_SNAPSHOT_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// A symlink's target is a single path string; 4 KiB is far beyond any real
/// target and keeps the stored object, the live walk, and the delta bounded.
pub const MAX_SYMLINK_TARGET_BYTES: usize = 4096;
/// The job snapshot's aggregate byte bound. A ~1 GiB repository is a supported
/// working size, so the bound is 2 GiB rather than 512 MiB.
pub const MAX_SNAPSHOT_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// The plan manifest carries every entry twice (files and index entries) plus
/// its status facts, so 20k files already approach 8 MiB. 32 MiB leaves room
/// for 50k files with long paths.
pub const MAX_SNAPSHOT_MANIFEST_BYTES: usize = 32 * 1024 * 1024;
/// The most sensitive-tracked warnings one plan lists. Beyond this bound the
/// plan stores one summary count fact instead of one fact per path, so a
/// repository full of credential-shaped files cannot flood the job's event
/// stream.
pub const MAX_SNAPSHOT_WARNINGS: usize = 64;
pub(crate) const MAX_PRIVATE_PREFIXES: usize = 128;
pub(crate) const MAX_PRIVATE_PREFIX_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotSelection {
    Head,
    CurrentWorkingContent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// A symlink whose stored content is its UTF-8 target path.
    Symlink,
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
            return Err(SnapshotError::unsafe_path());
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

/// A non-fatal fact about content the snapshot includes. It is not an
/// omission: the named path is in the snapshot. The reason is the shared
/// event contract, so the manifest and the job event carry one encoding.
pub use saya_types::SnapshotWarningReason;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotWarningFact {
    pub(super) path: SnapshotPath,
    pub(super) reason: SnapshotWarningReason,
}

impl SnapshotWarningFact {
    pub fn path(&self) -> &SnapshotPath {
        &self.path
    }

    pub fn reason(&self) -> SnapshotWarningReason {
        self.reason
    }
}

impl fmt::Debug for SnapshotWarningFact {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SnapshotWarningFact")
            .field("path", &"[snapshot path]")
            .field("reason", &self.reason)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
