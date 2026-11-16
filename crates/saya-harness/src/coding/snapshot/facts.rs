use std::fmt;

use serde::Serialize;

use super::types::SnapshotPath;

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct SnapshotStatusFact {
    pub(super) path: SnapshotPath,
    pub(super) original_path: Option<SnapshotPath>,
    pub(super) staged: bool,
    pub(super) unstaged: bool,
    pub(super) staged_added: bool,
    pub(super) unstaged_added: bool,
    pub(super) staged_deleted: bool,
    pub(super) unstaged_deleted: bool,
    pub(super) untracked: bool,
    pub(super) ignored: bool,
}

impl SnapshotStatusFact {
    pub fn path(&self) -> &SnapshotPath {
        &self.path
    }

    pub fn original_path(&self) -> Option<&SnapshotPath> {
        self.original_path.as_ref()
    }

    pub fn staged(&self) -> bool {
        self.staged
    }

    pub fn unstaged(&self) -> bool {
        self.unstaged
    }
}

impl fmt::Debug for SnapshotStatusFact {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SnapshotStatusFact")
            .field("path", &"[snapshot path]")
            .field("has_original_path", &self.original_path.is_some())
            .field("staged", &self.staged)
            .field("unstaged", &self.unstaged)
            .field("untracked", &self.untracked)
            .field("ignored", &self.ignored)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SnapshotChange {
    Tombstone {
        path: SnapshotPath,
    },
    Rename {
        from: SnapshotPath,
        to: SnapshotPath,
        staged: bool,
        unstaged: bool,
    },
}

impl SnapshotChange {
    pub fn is_tombstone(&self, path: &str) -> bool {
        matches!(self, Self::Tombstone { path: actual } if actual.as_str() == path)
    }

    pub fn is_rename(&self, from: &str, to: &str) -> bool {
        matches!(self, Self::Rename { from: actual_from, to: actual_to, .. }
            if actual_from.as_str() == from && actual_to.as_str() == to)
    }
}

impl fmt::Debug for SnapshotChange {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tombstone { .. } => formatter.write_str("Tombstone([snapshot path])"),
            Self::Rename {
                staged, unstaged, ..
            } => formatter
                .debug_struct("Rename")
                .field("from", &"[snapshot path]")
                .field("to", &"[snapshot path]")
                .field("staged", staged)
                .field("unstaged", unstaged)
                .finish(),
        }
    }
}
