use super::*;

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
