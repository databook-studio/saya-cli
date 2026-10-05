//! Quota and restore policy for edit checkpoints: the opt-in switch, the
//! session usage counters, admission before mutation, and whether a record
//! may currently restore its before-image. The record types live in `mod`;
//! the refusal kind lives in `error`.

use serde::{Deserialize, Serialize};

use super::{
    CheckpointRefusal, CheckpointState, EditCheckpoint, MAX_CHECKPOINT_FILE_BYTES,
    MAX_CHECKPOINT_SESSION_BYTES, MAX_CHECKPOINTS_PER_SESSION,
};

/// The opt-in switch for private recovery of direct file-tool edits.
/// Disabled by default: enabling recovery is a user decision, retention is
/// explained at that moment, and it is never enabled silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointPolicy {
    /// False keeps recovery off: edits proceed without checkpoints and no
    /// recovery data is retained.
    #[serde(default)]
    pub enabled: bool,
}

/// Running session usage against the checkpoint quotas: how many edits are
/// checkpointed and how many bytes their images retain. Callers pass this by
/// shared reference to [`admit`], which only reads it — refusing a quota
/// never evicts or rewrites existing history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CheckpointUsage {
    pub edits: u32,
    pub retained_bytes: u64,
}

/// Decides whether one more checkpoint may be admitted to a session, before
/// anything is mutated. `incoming_bytes` is the number of bytes the edit's
/// before and after images would newly retain — one checkpoint covers one
/// direct file-tool edit. Refusal is an error, never an eviction: usage is
/// only read, and existing checkpoints are never dropped to make room.
pub fn admit(usage: &CheckpointUsage, incoming_bytes: u64) -> Result<(), CheckpointRefusal> {
    if incoming_bytes > MAX_CHECKPOINT_FILE_BYTES {
        return Err(CheckpointRefusal::FileTooLarge);
    }
    if usage.edits >= MAX_CHECKPOINTS_PER_SESSION {
        return Err(CheckpointRefusal::TooManyCheckpoints);
    }
    let total = usage
        .retained_bytes
        .checked_add(incoming_bytes)
        .ok_or(CheckpointRefusal::SessionBytesExceeded)?;
    if total > MAX_CHECKPOINT_SESSION_BYTES {
        return Err(CheckpointRefusal::SessionBytesExceeded);
    }
    Ok(())
}

/// Whether a checkpoint may currently restore its before-image, and if not,
/// why not. Restore is conditional on the current digest matching the
/// recorded post-edit version, and only a committed record carries that
/// digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RestoreEligibility {
    /// The record is committed and the file's current digest still matches
    /// the recorded post-edit digest: applying the before-image would undo
    /// exactly this edit.
    Eligible,
    /// The record is still prepared: the edit is not known to have landed,
    /// so there is no post-edit version to match.
    NotCommitted,
    /// A crash or failure left it unknown whether the edit landed; the
    /// record is kept, never silently converted to committed or discarded.
    EffectUncertain,
    /// The file exists but its current digest no longer matches the recorded
    /// post-edit digest: the edit's effect has since changed, so undoing it
    /// is no longer sound.
    FileChangedSince,
    /// The recorded post-edit version exists but the file currently does not.
    FileMissing,
}

/// Assesses whether `record` may restore its before-image given the file's
/// current digest, if the file exists. Only a committed record is eligible,
/// and only while the file's current digest equals the recorded post-edit
/// digest — including for a create edit, whose `Absent` before-image means
/// restore would delete the file: the file must still match the post-edit
/// version first. A digest detects drift and does not authenticate.
pub fn restore_eligibility(
    record: &EditCheckpoint,
    current_sha256: Option<&str>,
) -> RestoreEligibility {
    match record.state {
        CheckpointState::Prepared => RestoreEligibility::NotCommitted,
        CheckpointState::Uncertain => RestoreEligibility::EffectUncertain,
        CheckpointState::Committed => {
            let Some(after) = &record.after else {
                // Unreachable for a valid record (validate refuses it): with
                // no post-edit digest recorded there is nothing to match.
                return RestoreEligibility::NotCommitted;
            };
            match current_sha256 {
                None => RestoreEligibility::FileMissing,
                Some(current) if current == after.sha256 => RestoreEligibility::Eligible,
                Some(_) => RestoreEligibility::FileChangedSince,
            }
        }
    }
}
