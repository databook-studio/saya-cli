//! The checkpoint lifecycle: `prepare_checkpoint` runs every gate — policy,
//! session identity and tombstone, screening, quota, record validation —
//! before any write, then stores the exact bytes first and the `Prepared`
//! record second, removing the bytes again if the record write fails. Commit
//! and the uncertain mark are read-modify-write transitions with idempotent
//! repeats, and nothing here ever evicts history to admit a new checkpoint.

use super::files::{
    ensure_dirs, id_taken, image_path, mint_id, sha256_hex, write_image, write_record,
};
use super::records::{loaded_record, rewrite};
use super::screen::admit_content;
use super::{CheckpointStoreError, NewCheckpoint, gated_session, invalid};
use crate::session_lifecycle::delete::remove_path_no_follow;
use crate::{FsSessionStore, StoreError};
use saya_types::{
    BeforeImage, CHECKPOINT_VERSION, CheckpointId, CheckpointPolicy, CheckpointState,
    EditCheckpoint, FileVersion, admit,
};

/// Builds the `Prepared` record for the edit — `Present` with the exact
/// bytes' digest and size, or `Absent` for a create — and validates it, so
/// a caller-supplied path the contract refuses fails here, before any
/// write.
fn prepared_record(
    id: String,
    session_id: &str,
    request: &NewCheckpoint<'_>,
    now_unix_ms: i64,
) -> Result<EditCheckpoint, CheckpointStoreError> {
    let before = match request.before {
        None => BeforeImage::Absent,
        Some(bytes) => BeforeImage::Present(FileVersion {
            sha256: sha256_hex(bytes),
            size: bytes.len() as u64,
        }),
    };
    let record = EditCheckpoint {
        version: CHECKPOINT_VERSION,
        id: CheckpointId::parse(&id)
            .map_err(|_| CheckpointStoreError::Store(StoreError::Unavailable))?,
        session_id: session_id.to_owned(),
        path: request.path.to_owned(),
        source: request.source.clone(),
        state: CheckpointState::Prepared,
        before,
        after: None,
        created_unix_ms: now_unix_ms,
        updated_unix_ms: now_unix_ms,
    };
    record.validate().map_err(|_| invalid())?;
    Ok(record)
}

impl FsSessionStore {
    /// Stores the before-image of one direct file-tool edit under a fresh
    /// checkpoint id, provided the policy is on, the session is live, the
    /// content passes screening, and the session quota admits it — every
    /// gate before any write. The bytes land first, the `Prepared` record
    /// second; a failed record write removes the bytes again.
    pub fn prepare_checkpoint(
        &self,
        session_id: &str,
        policy: &CheckpointPolicy,
        request: NewCheckpoint<'_>,
        now_unix_ms: i64,
    ) -> Result<EditCheckpoint, CheckpointStoreError> {
        if !policy.enabled {
            return Err(CheckpointStoreError::Disabled);
        }
        gated_session(self, session_id)?;
        admit_content(request.path, request.before).map_err(CheckpointStoreError::Screened)?;
        let usage = super::scan::scan_dir(self.sessions_root(), session_id)?.usage;
        admit(&usage, request.before.map_or(0, |bytes| bytes.len() as u64))
            .map_err(CheckpointStoreError::Refused)?;
        let mut record = prepared_record(mint_id(), session_id, &request, now_unix_ms)?;
        let dir = ensure_dirs(self.sessions_root(), session_id)?;
        let mut retries = 0;
        while id_taken(&dir, record.id.as_str())? {
            retries += 1;
            if retries == 16 {
                return Err(StoreError::Unavailable.into());
            }
            record = prepared_record(mint_id(), session_id, &request, now_unix_ms)?;
        }
        if let Some(bytes) = request.before {
            write_image(&dir, record.id.as_str(), bytes)?;
        }
        let bytes = record.to_json_bytes().map_err(|_| invalid())?;
        if write_record(&dir, record.id.as_str(), &bytes).is_err() {
            let _ = remove_path_no_follow(&image_path(&dir, record.id.as_str()));
            return Err(StoreError::Unavailable.into());
        }
        Ok(record)
    }

    /// Records that the edit landed, with the post-edit digest: only a
    /// `Prepared` record commits, and a `Committed` record repeats
    /// idempotently with the same after-image and is `Corrupt` otherwise.
    pub fn commit_checkpoint(
        &self,
        session_id: &str,
        id: &str,
        after: FileVersion,
        now_unix_ms: i64,
    ) -> Result<EditCheckpoint, CheckpointStoreError> {
        let (dir, mut record) = loaded_record(self, session_id, id)?;
        match record.state {
            CheckpointState::Committed => {
                if record.after.as_ref() == Some(&after) {
                    return Ok(record);
                }
                return Err(CheckpointStoreError::Corrupt);
            }
            CheckpointState::Prepared => {}
            _ => return Err(CheckpointStoreError::Corrupt),
        }
        record.state = CheckpointState::Committed;
        record.after = Some(after);
        rewrite(&dir, id, &mut record, now_unix_ms)
    }

    /// Records that a crash or failure left the edit's outcome unknown:
    /// `Prepared` becomes `Uncertain` (idempotent on `Uncertain`), and a
    /// committed record is never silently demoted.
    pub fn mark_checkpoint_uncertain(
        &self,
        session_id: &str,
        id: &str,
        now_unix_ms: i64,
    ) -> Result<EditCheckpoint, CheckpointStoreError> {
        let (dir, mut record) = loaded_record(self, session_id, id)?;
        match record.state {
            CheckpointState::Uncertain => return Ok(record),
            CheckpointState::Prepared => {}
            _ => return Err(CheckpointStoreError::Corrupt),
        }
        record.state = CheckpointState::Uncertain;
        rewrite(&dir, id, &mut record, now_unix_ms)
    }
}
