//! Restore the immutable plan; this grants no filesystem or runtime authority.
use super::{
    SnapshotError, SnapshotManifest,
    facts::{SnapshotChange, SnapshotStatusFact},
    manifest::{self, SnapshotManifestBody},
    types::*,
};
use saya_types::{ManagedCheckoutPath, Sha256Digest};
use serde::de::DeserializeOwned;
use serde_json::Value;

impl SnapshotManifest {
    /// `expected_digest` must come from trusted persisted ownership. A caller
    /// supplying its own bytes and digest has proved integrity, not approval.
    /// No original repository path is reopened or replanned here.
    pub fn from_verified_bytes(
        bytes: &[u8],
        expected_digest: &Sha256Digest,
    ) -> Result<Self, SnapshotError> {
        if bytes.len() > MAX_SNAPSHOT_MANIFEST_BYTES {
            return Err(SnapshotError::TooLarge);
        }
        if &manifest::digest(bytes)? != expected_digest {
            return Err(SnapshotError::ManifestDigestMismatch);
        }
        let value: Value = serde_json::from_slice(bytes).map_err(|_| invalid())?;
        let body = body(&value)?;
        // This rejects duplicate/unknown fields, alternate encodings and any
        // unvalidated source field: only the owning canonical shape survives.
        if manifest::manifest_bytes(&body)? != bytes {
            return Err(invalid());
        }
        Ok(Self {
            body,
            manifest_sha256: expected_digest.clone(),
            canonical_bytes: bytes.to_vec(),
        })
    }
}
fn invalid() -> SnapshotError {
    SnapshotError::CapturedManifestMalformed
}
fn field<T: DeserializeOwned>(value: &Value, key: &str) -> Result<T, SnapshotError> {
    serde_json::from_value(value.get(key).ok_or_else(invalid)?.clone()).map_err(|_| invalid())
}
fn array<'a>(value: &'a Value, key: &str) -> Result<&'a [Value], SnapshotError> {
    let items = value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    if items.len() > MAX_SNAPSHOT_FILES {
        return Err(SnapshotError::TooLarge);
    }
    Ok(items)
}
fn unique<'a>(paths: impl Iterator<Item = &'a SnapshotPath>) -> Result<(), SnapshotError> {
    let mut seen = std::collections::HashSet::new();
    for path in paths {
        if !seen.insert(path.as_str()) {
            return Err(invalid());
        }
    }
    Ok(())
}
fn body(value: &Value) -> Result<SnapshotManifestBody, SnapshotError> {
    let schema_version = field(value, "schema_version")?;
    if schema_version != 1 {
        return Err(invalid());
    }
    let repository_root: String = field(value, "repository_root")?;
    let common_dir: String = field(value, "common_dir")?;
    ManagedCheckoutPath::new(repository_root.clone()).map_err(|_| SnapshotError::UnsafePath)?;
    ManagedCheckoutPath::new(common_dir.clone()).map_err(|_| SnapshotError::UnsafePath)?;
    let base_commit: saya_types::GitObjectId = field(value, "base_commit")?;
    let selection = match value.get("selection").and_then(Value::as_str) {
        Some("head") => SnapshotSelection::Head,
        Some("current_working_content") => SnapshotSelection::CurrentWorkingContent,
        _ => return Err(invalid()),
    };
    let mut total = 0_u64;
    let mut entries = Vec::new();
    for item in array(value, "entries")? {
        let entry = entry(item, selection, base_commit.as_str().len())?;
        total = total
            .checked_add(entry.declared_bytes)
            .ok_or(SnapshotError::TooLarge)?;
        if total > MAX_SNAPSHOT_TOTAL_BYTES {
            return Err(SnapshotError::TooLarge);
        }
        entries.push(entry);
    }
    unique(entries.iter().map(|item| &item.path))?;
    let index_entries = array(value, "index_entries")?
        .iter()
        .map(index_entry)
        .collect::<Result<Vec<_>, _>>()?;
    unique(index_entries.iter().map(|item| &item.path))?;
    if index_entries
        .iter()
        .any(|entry| entry.object_id.as_str().len() != base_commit.as_str().len())
    {
        return Err(invalid());
    }
    let index_sha256 = field(value, "index_sha256")?;
    let index_bytes = serde_json::to_vec(&index_entries).map_err(|_| invalid())?;
    if manifest::digest(&index_bytes)? != index_sha256 {
        return Err(SnapshotError::ManifestDigestMismatch);
    }
    let status = array(value, "status")?
        .iter()
        .map(status)
        .collect::<Result<Vec<_>, _>>()?;
    unique(status.iter().map(|item| &item.path))?;
    let changes = array(value, "changes")?
        .iter()
        .map(change)
        .collect::<Result<Vec<_>, _>>()?;
    let omissions = array(value, "omissions")?
        .iter()
        .map(omission)
        .collect::<Result<Vec<_>, _>>()?;
    if omissions
        .iter()
        .any(|omission| entries.iter().any(|entry| entry.path == omission.path))
    {
        return Err(invalid());
    }
    Ok(SnapshotManifestBody {
        schema_version,
        snapshot_id: field(value, "snapshot_id")?,
        repository_id: field(value, "repository_id")?,
        common_dir_sha256: field(value, "common_dir_sha256")?,
        repository_root,
        common_dir,
        base_commit,
        selection,
        index_sha256,
        entries,
        index_entries,
        status,
        changes,
        omissions,
    })
}
fn entry(
    value: &Value,
    selection: SnapshotSelection,
    object_length: usize,
) -> Result<SnapshotFileEntry, SnapshotError> {
    let path: SnapshotPath = field(value, "path")?;
    if super::select_support::sensitive_path(path.as_str()) {
        return Err(SnapshotError::SensitiveTrackedPath);
    }
    let declared_bytes = field(value, "declared_bytes")?;
    if declared_bytes > MAX_SNAPSHOT_FILE_BYTES {
        return Err(SnapshotError::TooLarge);
    }
    let mode = field(value, "mode")?;
    let source = value.get("source").ok_or_else(invalid)?;
    let identity = value.get("source_identity").ok_or_else(invalid)?;
    let (source, source_identity) = match source.get("kind").and_then(Value::as_str) {
        Some("git_blob") if selection == SnapshotSelection::Head && identity.is_null() => {
            let object_id: saya_types::GitObjectId = field(source, "object_id")?;
            if object_id.as_str().len() != object_length {
                return Err(invalid());
            }
            (SnapshotContentSource::GitBlob { object_id }, None)
        }
        Some("working_tree")
            if selection == SnapshotSelection::CurrentWorkingContent && identity.is_object() =>
        {
            let source_identity = SnapshotSourceIdentity::new(
                field(identity, "device")?,
                field(identity, "inode")?,
                field(identity, "size")?,
                field(identity, "mode")?,
            );
            let executable = source_identity.mode & 0o111 != 0;
            if source_identity.size != declared_bytes
                || !matches!(source_identity.mode, 0o100644 | 0o100755)
                || executable != (mode == SnapshotFileMode::Executable)
            {
                return Err(invalid());
            }
            (SnapshotContentSource::WorkingTree, Some(source_identity))
        }
        _ => return Err(invalid()),
    };
    Ok(SnapshotFileEntry {
        path,
        source,
        mode,
        declared_bytes,
        source_identity,
    })
}
fn index_entry(value: &Value) -> Result<SnapshotIndexEntry, SnapshotError> {
    let entry = SnapshotIndexEntry {
        path: field(value, "path")?,
        mode: field(value, "mode")?,
        object_id: field(value, "object_id")?,
        stage: field(value, "stage")?,
        intent_to_add: field(value, "intent_to_add")?,
    };
    if entry.stage != 0 || !matches!(entry.mode, 0o100644 | 0o100755 | 0o120000) {
        return Err(SnapshotError::ConflictedIndex);
    }
    Ok(entry)
}
fn status(value: &Value) -> Result<SnapshotStatusFact, SnapshotError> {
    Ok(SnapshotStatusFact {
        path: field(value, "path")?,
        original_path: field(value, "original_path")?,
        staged: field(value, "staged")?,
        unstaged: field(value, "unstaged")?,
        staged_added: field(value, "staged_added")?,
        unstaged_added: field(value, "unstaged_added")?,
        staged_deleted: field(value, "staged_deleted")?,
        unstaged_deleted: field(value, "unstaged_deleted")?,
        untracked: field(value, "untracked")?,
        ignored: field(value, "ignored")?,
    })
}
fn change(value: &Value) -> Result<SnapshotChange, SnapshotError> {
    match value.get("kind").and_then(Value::as_str) {
        Some("tombstone") => Ok(SnapshotChange::Tombstone {
            path: field(value, "path")?,
        }),
        Some("rename") => Ok(SnapshotChange::Rename {
            from: field(value, "from")?,
            to: field(value, "to")?,
            staged: field(value, "staged")?,
            unstaged: field(value, "unstaged")?,
        }),
        _ => Err(invalid()),
    }
}
fn omission(value: &Value) -> Result<SnapshotOmissionFact, SnapshotError> {
    let reason = match value.get("reason").and_then(Value::as_str) {
        Some("ignored") => SnapshotOmissionReason::Ignored,
        Some("configured_private_path") => SnapshotOmissionReason::ConfiguredPrivatePath,
        Some("sensitive_untracked") => SnapshotOmissionReason::SensitiveUntracked,
        _ => return Err(invalid()),
    };
    Ok(SnapshotOmissionFact {
        path: field(value, "path")?,
        reason,
    })
}
