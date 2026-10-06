//! Tests for the recovery renderers: every sentence pinned, no digests or
//! file content outside the bounded diff text.

use super::*;
use saya_types::{
    CheckpointState, CheckpointUsage, EditSource, FileVersion, MAX_CHECKPOINT_SESSION_BYTES,
    MAX_CHECKPOINTS_PER_SESSION, RestoreEligibility,
};

fn item(n: u64, path: &str, source: EditSource, state: CheckpointState) -> ChangeItem {
    ChangeItem {
        id: format!("ck-{:032x}", n),
        path: path.to_owned(),
        source,
        state,
        before: BeforeSummary::Present { size: 4 },
        after: Some(FileVersion {
            sha256: format!("{:064x}", n),
            size: 6,
        }),
        created_unix_ms: 10_000,
        restore: StatusNow::Known(RestoreEligibility::Eligible),
    }
}

#[test]
fn the_listing_pins_the_header_and_every_status_sentence() {
    let mut items = vec![
        item(
            1,
            "notes.txt",
            EditSource::WorkspaceEdit,
            CheckpointState::Committed,
        ),
        item(
            2,
            "notes.txt",
            EditSource::WorkspaceEdit,
            CheckpointState::Committed,
        ),
        item(
            3,
            "gone.txt",
            EditSource::WorkspaceWrite,
            CheckpointState::Committed,
        ),
    ];
    items[1].restore = StatusNow::Known(RestoreEligibility::FileChangedSince);
    items[2].restore = StatusNow::Known(RestoreEligibility::FileMissing);
    items[2].path = "gone.txt".to_owned();

    let mut prepared = item(
        4,
        "prepared.txt",
        EditSource::WorkspaceWrite,
        CheckpointState::Prepared,
    );
    prepared.before = BeforeSummary::Absent;
    prepared.after = None;
    prepared.created_unix_ms = 70_000;
    prepared.restore = StatusNow::Known(RestoreEligibility::NotCommitted);
    let mut uncertain = item(
        5,
        "uncertain.txt",
        EditSource::WorkspaceEdit,
        CheckpointState::Uncertain,
    );
    uncertain.after = None;
    uncertain.created_unix_ms = 189_500;
    uncertain.restore = StatusNow::Known(RestoreEligibility::EffectUncertain);
    let mut missing = item(
        6,
        "adirectory",
        EditSource::WorkspaceEdit,
        CheckpointState::Committed,
    );
    missing.restore = StatusNow::Known(RestoreEligibility::FileMissing);
    missing.created_unix_ms = 100_000;
    let mut unreadable = item(
        7,
        "locked.txt",
        EditSource::WorkspaceEdit,
        CheckpointState::Committed,
    );
    unreadable.restore = StatusNow::Unreadable;
    unreadable.created_unix_ms = 1_000;
    let mut not_checked = item(
        8,
        "late.txt",
        EditSource::WorkspaceEdit,
        CheckpointState::Committed,
    );
    not_checked.restore = StatusNow::NotChecked;
    not_checked.created_unix_ms = 40_000;
    items.extend([prepared, uncertain, missing, unreadable, not_checked]);

    let view = ChangesView {
        items,
        usage: CheckpointUsage {
            edits: 8,
            retained_bytes: 32,
        },
        max_edits: MAX_CHECKPOINTS_PER_SESSION,
        max_bytes: MAX_CHECKPOINT_SESSION_BYTES,
    };
    assert_eq!(
        render_changes(&view, 190_000),
        concat!(
            "Recorded edits this session (8 of 128 · 1 KiB of 64 MiB):\n",
            "ck-00000000000000000000000000000001 notes.txt workspace_edit committed present 4 B -> 6 B (sha 00000000) · 3m ago · restorable now: the file still matches the recorded edit\n",
            "ck-00000000000000000000000000000002 notes.txt workspace_edit committed present 4 B -> 6 B (sha 00000000) · 3m ago · notes.txt changed after that edit was recorded, so restoring would overwrite newer work. Restore the newer edit first, or edit the file yourself.\n",
            "ck-00000000000000000000000000000003 gone.txt workspace_write committed present 4 B -> 6 B (sha 00000000) · 3m ago · gone.txt no longer exists, so there is nothing to restore over.\n",
            "ck-00000000000000000000000000000004 prepared.txt workspace_write prepared absent -> absent · 2m ago · That edit never completed, so there is nothing to restore.\n",
            "ck-00000000000000000000000000000005 uncertain.txt workspace_edit uncertain present 4 B -> absent · just now · That edit's outcome is uncertain (the session ended mid-edit), so it cannot be restored automatically.\n",
            "ck-00000000000000000000000000000006 adirectory workspace_edit committed present 4 B -> 6 B (sha 00000000) · 1m ago · adirectory no longer exists, so there is nothing to restore over.\n",
            "ck-00000000000000000000000000000007 locked.txt workspace_edit committed present 4 B -> 6 B (sha 00000000) · 3m ago · cannot read the file now, so its restore status is unknown\n",
            "ck-00000000000000000000000000000008 late.txt workspace_edit committed present 4 B -> 6 B (sha 00000000) · 2m ago · restore status not checked (the listing refreshes at most 32 files)",
        ),
        "every line and sentence is pinned; ages come from created_unix_ms"
    );
}

#[test]
fn every_restore_refusal_sentence_is_pinned() {
    let refused = |refusal| render_restore(&RestoreOutcome::Refused(refusal));
    assert_eq!(
        refused(RestoreRefusal::NotFound),
        (
            false,
            "No recorded edit carries that checkpoint id; nothing was changed.".to_owned()
        )
    );
    assert_eq!(
        refused(RestoreRefusal::NoWorkspace),
        (
            false,
            "No workspace is bound, so edits cannot be restored.".to_owned()
        )
    );
    assert_eq!(
        refused(RestoreRefusal::NotEligible {
            status: StatusNow::Known(RestoreEligibility::FileChangedSince),
            path: "notes.txt".to_owned(),
        }),
        (
            false,
            "notes.txt changed after that edit was recorded, so restoring would overwrite \
             newer work. Restore the newer edit first, or edit the file yourself."
                .to_owned()
        )
    );
    assert_eq!(
        refused(RestoreRefusal::NotEligible {
            status: StatusNow::Known(RestoreEligibility::FileMissing),
            path: "notes.txt".to_owned(),
        }),
        (
            false,
            "notes.txt no longer exists, so there is nothing to restore over.".to_owned()
        )
    );
    assert_eq!(
        refused(RestoreRefusal::NotEligible {
            status: StatusNow::Known(RestoreEligibility::EffectUncertain),
            path: "notes.txt".to_owned(),
        }),
        (
            false,
            "That edit's outcome is uncertain (the session ended mid-edit), so it cannot \
             be restored automatically."
                .to_owned()
        )
    );
    assert_eq!(
        refused(RestoreRefusal::NotEligible {
            status: StatusNow::Known(RestoreEligibility::NotCommitted),
            path: "notes.txt".to_owned(),
        }),
        (
            false,
            "That edit never completed, so there is nothing to restore.".to_owned()
        )
    );
    assert_eq!(
        refused(RestoreRefusal::NotEligible {
            status: StatusNow::Unreadable,
            path: "notes.txt".to_owned(),
        }),
        (
            false,
            "The file cannot be read right now, so the restore was refused; nothing was changed."
                .to_owned()
        )
    );
    assert_eq!(
        refused(RestoreRefusal::BeforeImageUnavailable),
        (
            false,
            "The saved before-image is no longer in recovery storage; the file was not changed."
                .to_owned()
        )
    );
    assert_eq!(
        refused(RestoreRefusal::BeforeImageCorrupt),
        (
            false,
            "The saved before-image failed its integrity check; the file was not changed."
                .to_owned()
        )
    );
    assert_eq!(
        refused(RestoreRefusal::ChangedWhileRestoring),
        (
            false,
            "The file changed while restoring; nothing was changed.".to_owned()
        )
    );
    assert_eq!(
        refused(RestoreRefusal::Failed("disk on fire".to_owned())),
        (
            false,
            "The restore failed (disk on fire); nothing was changed.".to_owned()
        )
    );
}

#[test]
fn the_restore_success_sentences_are_pinned() {
    let id = format!("ck-{:032x}", 9);
    assert_eq!(
        render_restore(&RestoreOutcome::Restored {
            id: id.clone(),
            path: "notes.txt".to_owned(),
            removed_checkpoint: true,
        }),
        (
            true,
            format!("Restored notes.txt to its state before the edit (checkpoint {id} removed).")
        )
    );
    assert_eq!(
        render_restore(&RestoreOutcome::Restored {
            id: id.clone(),
            path: "notes.txt".to_owned(),
            removed_checkpoint: false,
        }),
        (
            true,
            format!(
                "Restored notes.txt to its state before the edit, but checkpoint {id} \
                 could not be removed."
            )
        )
    );
    assert_eq!(
        render_restore(&RestoreOutcome::Restored {
            id,
            path: "sub/made.txt".to_owned(),
            removed_checkpoint: true,
        }),
        (
            true,
            "Restored sub/made.txt to its state before the edit \
             (checkpoint ck-00000000000000000000000000000009 removed). \
             The parent directory was not removed."
                .to_owned()
        )
    );
}

#[test]
fn the_diff_outcome_texts_are_pinned() {
    assert_eq!(
        render_diff_outcome(&DiffOutcome::Diff {
            text: "diff preview: f.txt".to_owned(),
            truncated: false,
        }),
        (true, "diff preview: f.txt".to_owned())
    );
    assert_eq!(
        render_diff_outcome(&DiffOutcome::NothingToDiff),
        (
            true,
            "No difference: the file already matches its recorded before state.".to_owned()
        )
    );
    assert_eq!(
        render_diff_outcome(&DiffOutcome::BinaryOrNonText {
            before_bytes: 3,
            current_bytes: 5,
        }),
        (
            true,
            "Not diffable: the file is binary or non-text (before 3 B, current 5 B).".to_owned()
        )
    );
    assert_eq!(
        render_diff_outcome(&DiffOutcome::TooLargeToDiff {
            before_bytes: 70_000,
            current_bytes: 10,
        }),
        (
            true,
            "Too large to diff (before 70000 B, current 10 B); the preview is limited \
             to 64 KiB per side."
                .to_owned()
        )
    );
    assert_eq!(
        render_diff_outcome(&DiffOutcome::Refused(RestoreRefusal::NoWorkspace)),
        (
            false,
            "No workspace is bound, so edits cannot be restored.".to_owned()
        )
    );
}
