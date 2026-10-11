//! One bound source observation for apply-back and recovery: root inode and target modes.

use super::{
    DeltaKind,
    transaction::ApplyBackInput,
    transaction_journal::{JournalError, sha_hex},
};
use crate::{coding::snapshot::MAX_SNAPSHOT_FILES, workspace::Workspace};
use serde_json::json;
use std::{collections::BTreeMap, num::NonZeroU64};

pub(super) const MAX_RECOVERY_JOURNAL_BYTES: u64 = 8 * 1024 * 1024;

pub(super) struct BoundOperation {
    pub(super) digest: String,
    pub(super) target_modes: BTreeMap<String, u32>,
    pub(super) baseline_modes: BTreeMap<String, u32>,
    body: serde_json::Value,
}

pub(super) fn operation_binding(
    input: &ApplyBackInput<'_>,
    sequence: NonZeroU64,
    original: &Workspace,
    recorded_modes: Option<&BTreeMap<String, u32>>,
) -> Result<BoundOperation, JournalError> {
    if input.accepted.len() > MAX_SNAPSHOT_FILES {
        return Err(JournalError::OverBudget);
    }
    let mut entries = Vec::new();
    let mut image_bytes = 0u64;
    let mut delta_bytes = 0u64;
    let mut target_modes = BTreeMap::new();
    let mut baseline_modes = BTreeMap::new();
    for accepted in input.accepted {
        let child = Workspace::open(std::path::Path::new(accepted.verified().path().as_str()))
            .map_err(|_| JournalError::Io)?;
        for entry in accepted.delta().entries() {
            if entries.len() >= MAX_SNAPSHOT_FILES {
                return Err(JournalError::OverBudget);
            }
            delta_bytes = delta_bytes
                .checked_add(entry.bytes())
                .ok_or(JournalError::OverBudget)?;
            if delta_bytes > crate::coding::snapshot::MAX_SNAPSHOT_TOTAL_BYTES {
                return Err(JournalError::OverBudget);
            }
            if entry.kind() != DeltaKind::Added {
                let base = input
                    .baseline
                    .entries()
                    .iter()
                    .find(|base| base.path().as_str() == entry.path())
                    .ok_or(JournalError::Corrupt)?;
                image_bytes = image_bytes
                    .checked_add(base.captured_bytes())
                    .ok_or(JournalError::OverBudget)?;
                if image_bytes > super::transaction_images::APPLY_BACK_IMAGE_TOTAL_BYTES {
                    return Err(JournalError::OverBudget);
                }
            }
            let original_mode = if entry.kind() == DeltaKind::Added {
                None
            } else {
                let base = input
                    .baseline
                    .entries()
                    .iter()
                    .find(|base| base.path().as_str() == entry.path())
                    .ok_or(JournalError::Corrupt)?;
                let mode = match recorded_modes {
                    Some(modes) => *modes.get(entry.path()).ok_or(JournalError::Corrupt)?,
                    None => super::apply_fs::file_mode(original, entry.path())
                        .map_err(|_| JournalError::Io)?,
                };
                if mode > 0o777
                    || (mode & 0o111 != 0)
                        != (base.mode() == crate::coding::snapshot::SnapshotFileMode::Executable)
                {
                    return Err(JournalError::Corrupt);
                }
                baseline_modes.insert(entry.path().to_owned(), mode);
                Some(mode)
            };
            let child_mode = if entry.kind() == DeltaKind::Deleted {
                None
            } else {
                Some(
                    super::apply_fs::file_mode(&child, entry.path())
                        .map_err(|_| JournalError::Io)?,
                )
            };
            let target_mode = child_mode.map(|child| match original_mode {
                None => child,
                Some(base) => {
                    (base & !0o111)
                        | if child & 0o111 != 0 {
                            (base & 0o444) >> 2
                        } else {
                            0
                        }
                }
            });
            if let Some(target) = target_mode {
                target_modes.insert(entry.path().to_owned(), target);
            }
            entries.push(json!([
                accepted.verified().checkout_id().as_str(),
                entry.path(),
                kind(entry.kind()),
                entry.bytes(),
                entry.content_sha256().map(|sha| sha.as_str()),
                child_mode,
                target_mode,
                original_mode
            ]));
        }
    }
    if recorded_modes.is_some_and(|modes| modes.len() != baseline_modes.len()) {
        return Err(JournalError::Corrupt);
    }
    let body = json!({
        "job": input.job_id.as_str(), "operation": sequence.get(),
        "candidate": input.candidate.checkout_id().as_str(),
        "repository": input.baseline.repository_id().as_str(),
        "snapshot": input.baseline.snapshot_id().as_str(),
        "baseline": input.baseline.captured_manifest_sha256().as_str(),
        "plan": input.plan.digest().as_str(), "entries": entries,
        "original_root": root_identity(original)?,
    });
    let bytes = serde_json::to_vec(&body).map_err(|_| JournalError::Io)?;
    if bytes.len() as u64 > MAX_RECOVERY_JOURNAL_BYTES {
        return Err(JournalError::OverBudget);
    }
    Ok(BoundOperation {
        digest: sha_hex(&bytes),
        target_modes,
        baseline_modes,
        body,
    })
}

#[cfg(unix)]
pub(super) fn root_identity(original: &Workspace) -> Result<[u64; 2], JournalError> {
    use std::os::unix::fs::MetadataExt;
    let fd = original
        .root_dir_fd()
        .try_clone()
        .map_err(|_| JournalError::Io)?;
    let meta = std::fs::File::from(fd)
        .metadata()
        .map_err(|_| JournalError::Io)?;
    Ok([meta.dev(), meta.ino()])
}

#[cfg(not(unix))]
pub(super) fn root_identity(_original: &Workspace) -> Result<[u64; 2], JournalError> {
    Err(JournalError::Io)
}

fn kind(kind: DeltaKind) -> &'static str {
    match kind {
        DeltaKind::Added => "added",
        DeltaKind::Modified => "modified",
        DeltaKind::Deleted => "deleted",
    }
}

impl BoundOperation {
    pub(super) fn for_sequence(mut self, sequence: NonZeroU64) -> Result<Self, JournalError> {
        self.body["operation"] = serde_json::Value::from(sequence.get());
        self.digest = sha_hex(&serde_json::to_vec(&self.body).map_err(|_| JournalError::Io)?);
        Ok(self)
    }
}

/// Reserve every legal progress and rollback record before opening the store intent.
pub(super) fn check_journal_budget(
    input: &ApplyBackInput<'_>,
    bound: &BoundOperation,
) -> Result<(), JournalError> {
    use super::{transaction_journal::JournalEvent, transaction_journal_io::encode_event};
    let begin = JournalEvent::Begin {
        binding: bound.digest.clone(),
        baseline_modes: bound.baseline_modes.clone(),
    };
    let mut bytes = encode_event(&begin)?.len() as u64;
    for accepted in input.accepted {
        for entry in accepted.delta().entries() {
            let image = input
                .baseline
                .entries()
                .iter()
                .find(|base| base.path().as_str() == entry.path())
                .map(|base| base.content_sha256().as_str().to_owned());
            let intent = super::recover_journal::intent_event(input, entry, image);
            let path = entry.path().to_owned();
            for event in [
                intent,
                JournalEvent::Done {
                    path: path.clone(),
                    result: entry.content_sha256().map(|sha| sha.as_str().to_owned()),
                },
                JournalEvent::Restoring { path: path.clone() },
                JournalEvent::Restored { path },
            ] {
                bytes = bytes
                    .checked_add(encode_event(&event)?.len() as u64)
                    .ok_or(JournalError::OverBudget)?;
                if bytes > MAX_RECOVERY_JOURNAL_BYTES {
                    return Err(JournalError::OverBudget);
                }
            }
        }
    }
    if bytes > MAX_RECOVERY_JOURNAL_BYTES {
        return Err(JournalError::OverBudget);
    }
    Ok(())
}
