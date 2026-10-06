//! The restore preview: a bounded unified-style diff between the recorded
//! before-image and the file as it stands now, over explicit inputs. The
//! byte gathering is the store/workspace side; [`render_diff`] itself is
//! pure and deterministic.

use super::hunks;
use super::line_diff;
use super::{DiffOutcome, RestoreRefusal};
use saya_harness::workspace::Workspace;
use saya_store::{CheckpointStoreError, FsSessionStore};
use saya_types::{BeforeImage, CheckpointId, MAX_CHECKPOINT_FILE_BYTES};

pub(crate) use hunks::DIFF_MAX_OUTPUT_LINES;
pub(crate) use line_diff::{DIFF_MAX_BYTES, DIFF_MAX_CELLS, DIFF_MAX_LINES};

const CREATED_NOTE: &str = "before is empty (created by the edit)";
const MISSING_NOTE: &str = "current is missing (file is missing now)";

/// Previews what restoring `id` would change: the recorded before-image
/// against the file as it stands now. A create edit previews against an
/// empty before side and says so; a file that is missing now previews
/// against emptiness and says so.
pub(crate) fn diff_change(
    store: &FsSessionStore,
    session_id: &str,
    workspace: Option<&Workspace>,
    id: &str,
) -> DiffOutcome {
    let Some(workspace) = workspace else {
        return DiffOutcome::Refused(RestoreRefusal::NoWorkspace);
    };
    let Ok(parsed) = CheckpointId::parse(id) else {
        return DiffOutcome::Refused(RestoreRefusal::NotFound);
    };
    let records = match store.list_checkpoints(session_id) {
        Ok(records) => records,
        Err(error) => return DiffOutcome::Refused(RestoreRefusal::Failed(error.to_string())),
    };
    let Some(record) = records.into_iter().find(|record| record.id == parsed) else {
        return DiffOutcome::Refused(RestoreRefusal::NotFound);
    };
    let created = record.before == BeforeImage::Absent;
    let before_bytes = if created {
        Vec::new()
    } else {
        match store.read_before_image(session_id, id) {
            Ok(bytes) => bytes,
            Err(CheckpointStoreError::NotFound) => {
                return DiffOutcome::Refused(RestoreRefusal::BeforeImageUnavailable);
            }
            Err(CheckpointStoreError::Corrupt) => {
                return DiffOutcome::Refused(RestoreRefusal::BeforeImageCorrupt);
            }
            Err(error) => return DiffOutcome::Refused(RestoreRefusal::Failed(error.to_string())),
        }
    };
    let current = match workspace.read(&record.path, MAX_CHECKPOINT_FILE_BYTES) {
        Ok(read) => Some(read.bytes),
        Err(error) if error.is_not_found() => None,
        Err(error) => return DiffOutcome::Refused(RestoreRefusal::Failed(error.to_string())),
    };
    let missing = current.is_none();
    match render_diff(&before_bytes, current.as_deref(), &record.path) {
        DiffOutcome::Diff { text, truncated } => {
            let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
            let mut at = 3usize;
            if created {
                lines.insert(at, CREATED_NOTE.to_owned());
                at += 1;
            }
            if missing {
                lines.insert(at, MISSING_NOTE.to_owned());
            }
            DiffOutcome::Diff {
                text: lines.join("\n"),
                truncated,
            }
        }
        other => other,
    }
}

/// The pure bounded diff between `before` and the current bytes (`None`
/// when the file is missing now, treated as empty). Unified-style text
/// with hard caps on bytes, lines, table cells, and output; never more
/// output than the bounds allow, whatever the input shape.
pub(crate) fn render_diff(before: &[u8], current: Option<&[u8]>, path: &str) -> DiffOutcome {
    let current = current.unwrap_or(&[]);
    let too_large = || DiffOutcome::TooLargeToDiff {
        before_bytes: before.len(),
        current_bytes: current.len(),
    };
    if before.len() > DIFF_MAX_BYTES || current.len() > DIFF_MAX_BYTES {
        return too_large();
    }
    let (Some(before_text), Some(current_text)) =
        (line_diff::text_side(before), line_diff::text_side(current))
    else {
        return DiffOutcome::BinaryOrNonText {
            before_bytes: before.len(),
            current_bytes: current.len(),
        };
    };
    let (before_lines, before_trailing) = line_diff::split_lines(before_text);
    let (current_lines, current_trailing) = line_diff::split_lines(current_text);
    if before_lines.len() > DIFF_MAX_LINES || current_lines.len() > DIFF_MAX_LINES {
        return too_large();
    }
    if (before_lines.len() + 1).saturating_mul(current_lines.len() + 1) > DIFF_MAX_CELLS {
        return too_large();
    }
    if before == current {
        return DiffOutcome::NothingToDiff;
    }
    let mut ops = line_diff::lcs_ops(&before_lines, &current_lines);
    if ops.iter().all(|op| matches!(op, line_diff::Op::Same(_))) {
        if before_trailing == current_trailing {
            return DiffOutcome::NothingToDiff;
        }
        // Only the final newline differs: show it on the last lines.
        let last = ops.len() - 1;
        ops[last] = line_diff::Op::Del(before_lines.len() - 1);
        ops.push(line_diff::Op::Ins(current_lines.len() - 1));
    }
    let (text, truncated) = hunks::emit(
        path,
        &before_lines,
        &current_lines,
        &ops,
        before_trailing,
        current_trailing,
    );
    DiffOutcome::Diff { text, truncated }
}
