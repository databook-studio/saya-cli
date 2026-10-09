//! Contract tests for edit checkpoint records and their recovery policy in
//! `saya_types`.
//!
//! A checkpoint is opt-in private recovery data for direct file-tool edits,
//! read back from storage — model-adjacent, untrusted input — so `validate()`
//! re-checks every bound no matter how the record arrived: the version, the
//! digests, the file sizes, the path shape, the timestamps, and the
//! state/after-image identity rules. It must be impossible to represent a
//! subprocess, network, scratch, or database rollback as a checkpoint, restore
//! is only eligible for a committed edit whose post-edit digest still matches
//! the file, and a quota refusal happens before mutation without evicting
//! history.

use saya_types::{
    BeforeImage, CHECKPOINT_VERSION, CheckpointError, CheckpointId, CheckpointPolicy,
    CheckpointRefusal, CheckpointState, CheckpointUsage, EditCheckpoint, EditSource, FileVersion,
    MAX_CHECKPOINT_FILE_BYTES, MAX_CHECKPOINT_PATH_BYTES, MAX_CHECKPOINT_RECORD_BYTES,
    MAX_CHECKPOINT_SESSION_BYTES, MAX_CHECKPOINTS_PER_SESSION, RestoreEligibility, admit,
    restore_eligibility,
};

const BEFORE_SHA256: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const AFTER_SHA256: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
const ID: &str = "ck-00112233445566778899aabbccddeeff";
const SESSION: &str = "sess-1";
const CREATED: i64 = 1_700_000_000_000;

fn id() -> CheckpointId {
    CheckpointId::parse(ID).unwrap()
}

fn version(sha: &str, size: u64) -> FileVersion {
    FileVersion {
        sha256: sha.to_string(),
        size,
    }
}

fn record(state: CheckpointState, before: BeforeImage) -> EditCheckpoint {
    let after = (state == CheckpointState::Committed).then(|| version(AFTER_SHA256, 2048));
    EditCheckpoint {
        version: CHECKPOINT_VERSION,
        id: id(),
        session_id: SESSION.to_string(),
        path: "src/main.rs".to_string(),
        source: EditSource::WorkspaceEdit,
        state,
        before,
        after,
        created_unix_ms: CREATED,
        updated_unix_ms: CREATED,
    }
}

fn prepared() -> EditCheckpoint {
    record(
        CheckpointState::Prepared,
        BeforeImage::Present(version(BEFORE_SHA256, 1024)),
    )
}

fn committed() -> EditCheckpoint {
    record(
        CheckpointState::Committed,
        BeforeImage::Present(version(BEFORE_SHA256, 1024)),
    )
}

fn uncertain() -> EditCheckpoint {
    record(
        CheckpointState::Uncertain,
        BeforeImage::Present(version(BEFORE_SHA256, 1024)),
    )
}

fn create_committed() -> EditCheckpoint {
    record(CheckpointState::Committed, BeforeImage::Absent)
}

fn bytes_of(record: &EditCheckpoint) -> Vec<u8> {
    serde_json::to_vec(record).unwrap()
}

fn usage(edits: u32, retained_bytes: u64) -> CheckpointUsage {
    CheckpointUsage {
        edits,
        retained_bytes,
    }
}

#[path = "edit_checkpoint/path_and_identity.rs"]
mod path_and_identity;
#[path = "edit_checkpoint/policy_and_size.rs"]
mod policy_and_size;
#[path = "edit_checkpoint/quotas.rs"]
mod quotas;
#[path = "edit_checkpoint/record_wire.rs"]
mod record_wire;
#[path = "edit_checkpoint/restore.rs"]
mod restore;
#[path = "edit_checkpoint/state_and_digest.rs"]
mod state_and_digest;
