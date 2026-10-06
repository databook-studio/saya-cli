//! The restore side of recovery: undo one recorded edit by putting back
//! the exact before bytes — or removing what the edit created — but only
//! when the file still is, digest for digest, the recorded post-edit
//! state. The harness's guarded operations verify that digest under their
//! commit gate and refuse symlinks, escapes, and oversize files; the
//! model-tool redaction guard does not apply here, because a restore
//! writes the user's own recorded bytes through the same gate. A restore
//! never claims to undo subprocess, network, scratch, or database effects.

use super::{RestoreOutcome, RestoreRefusal, StatusNow, sha256_hex};
use saya_harness::HarnessError;
use saya_harness::workspace::Workspace;
use saya_store::{CheckpointStoreError, FsSessionStore};
use saya_types::{BeforeImage, CheckpointId, RestoreEligibility, restore_eligibility};

/// Restores one recorded edit of the session. Nothing is mutated unless
/// the file's current digest still matches the recorded post-edit state;
/// a successful restore consumes the checkpoint by deleting it, and a
/// failed deletion still reports the file restored.
pub(crate) fn restore_change(
    store: &FsSessionStore,
    session_id: &str,
    workspace: Option<&Workspace>,
    id: &str,
) -> RestoreOutcome {
    let Some(workspace) = workspace else {
        return RestoreOutcome::Refused(RestoreRefusal::NoWorkspace);
    };
    let Ok(parsed) = CheckpointId::parse(id) else {
        return RestoreOutcome::Refused(RestoreRefusal::NotFound);
    };
    let records = match store.list_checkpoints(session_id) {
        Ok(records) => records,
        Err(error) => return RestoreOutcome::Refused(RestoreRefusal::Failed(error.to_string())),
    };
    let Some(record) = records.into_iter().find(|record| record.id == parsed) else {
        return RestoreOutcome::Refused(RestoreRefusal::NotFound);
    };
    // The file must still be the recorded post-edit state. A symlinked
    // path component refuses here with the harness's own reason: the
    // guarded operation below would refuse it the same way.
    let status = match workspace.read(&record.path, 0) {
        Ok(read) => StatusNow::Known(restore_eligibility(&record, Some(&read.digest))),
        Err(error) if error.is_not_found() => StatusNow::Known(restore_eligibility(&record, None)),
        Err(error @ HarnessError::SymlinkRefused { .. }) => {
            return RestoreOutcome::Refused(RestoreRefusal::Failed(error.to_string()));
        }
        Err(_) => StatusNow::Unreadable,
    };
    let path = record.path.clone();
    let StatusNow::Known(RestoreEligibility::Eligible) = status else {
        return RestoreOutcome::Refused(RestoreRefusal::NotEligible { status, path });
    };
    let Some(after) = record.after.clone() else {
        return RestoreOutcome::Refused(RestoreRefusal::NotEligible {
            status: StatusNow::Known(RestoreEligibility::NotCommitted),
            path,
        });
    };
    let applied = match &record.before {
        BeforeImage::Present(before_version) => match store.read_before_image(session_id, id) {
            Ok(bytes) => {
                if sha256_hex(&bytes) != before_version.sha256
                    || bytes.len() as u64 != before_version.size
                {
                    return RestoreOutcome::Refused(RestoreRefusal::BeforeImageCorrupt);
                }
                workspace.write_guarded(&record.path, &after.sha256, &bytes)
            }
            Err(CheckpointStoreError::NotFound) => {
                return RestoreOutcome::Refused(RestoreRefusal::BeforeImageUnavailable);
            }
            Err(CheckpointStoreError::Corrupt) => {
                return RestoreOutcome::Refused(RestoreRefusal::BeforeImageCorrupt);
            }
            Err(error) => {
                return RestoreOutcome::Refused(RestoreRefusal::Failed(error.to_string()));
            }
        },
        BeforeImage::Absent => workspace.remove_guarded(&record.path, &after.sha256),
        // A before-image of an unrecognized kind means a newer record
        // version; nothing about it can be applied safely.
        _ => {
            return RestoreOutcome::Refused(RestoreRefusal::Failed(
                "the record's before-image is of an unrecognized kind".to_owned(),
            ));
        }
    };
    match applied {
        Ok(()) => {}
        Err(HarnessError::ContentChanged { .. }) => {
            return RestoreOutcome::Refused(RestoreRefusal::ChangedWhileRestoring);
        }
        Err(error) => return RestoreOutcome::Refused(RestoreRefusal::Failed(error.to_string())),
    }
    let removed_checkpoint = store.delete_checkpoint(session_id, id).is_ok();
    RestoreOutcome::Restored {
        id: record.id.as_str().to_owned(),
        path,
        removed_checkpoint,
    }
}
