use super::*;

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
