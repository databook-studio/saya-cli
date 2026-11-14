//! Plain-text rendering for the restore and diff outcomes: one decision
//! per line, a pinned sentence for every state. The refusal sentences are
//! the shared source for the restore surface and the matching listing
//! statuses, so the two can never disagree about why.

use super::{DiffOutcome, RestoreOutcome, RestoreRefusal, StatusNow};
use saya_types::RestoreEligibility;

/// The pinned sentence for one refused restore.
pub(crate) fn restore_refusal_text(refusal: &RestoreRefusal) -> String {
    match refusal {
        RestoreRefusal::NotFound => {
            "No recorded edit carries that checkpoint id; nothing was changed.".to_owned()
        }
        RestoreRefusal::NoWorkspace => {
            "No workspace is bound, so edits cannot be restored.".to_owned()
        }
        RestoreRefusal::NotEligible { status, path } => match status {
            StatusNow::Known(RestoreEligibility::Eligible) => {
                "The file still matches the recorded edit, so it can be restored.".to_owned()
            }
            StatusNow::Known(RestoreEligibility::NotCommitted) => {
                "That edit never completed, so there is nothing to restore.".to_owned()
            }
            StatusNow::Known(RestoreEligibility::EffectUncertain) => {
                "That edit's outcome is uncertain (the session ended mid-edit), so it \
                 cannot be restored automatically."
                    .to_owned()
            }
            StatusNow::Known(RestoreEligibility::FileChangedSince) => format!(
                "{path} changed after that edit was recorded, so restoring would overwrite \
                 newer work. Restore the newer edit first, or edit the file yourself."
            ),
            StatusNow::Known(RestoreEligibility::FileMissing) => {
                format!("{path} no longer exists, so there is nothing to restore over.")
            }
            StatusNow::Known(_) => "The recorded state is not restorable.".to_owned(),
            StatusNow::Unreadable => {
                "The file cannot be read right now, so the restore was refused; \
                 nothing was changed."
                    .to_owned()
            }
            StatusNow::NotChecked => {
                "The restore status was not checked; nothing was changed.".to_owned()
            }
        },
        RestoreRefusal::BeforeImageUnavailable => {
            "The saved before-image is no longer in recovery storage; the file was not changed."
                .to_owned()
        }
        RestoreRefusal::BeforeImageCorrupt => {
            "The saved before-image failed its integrity check; the file was not changed."
                .to_owned()
        }
        RestoreRefusal::ChangedWhileRestoring => {
            "The file changed while restoring; nothing was changed.".to_owned()
        }
        RestoreRefusal::Failed(error) => {
            format!("The restore failed ({error}); nothing was changed.")
        }
    }
}

/// Renders a restore outcome as (ok, message). Removing a created file
/// never removes its (possibly now-empty) parent directory, and the
/// success text says so whenever the path had a directory part.
pub(crate) fn render_restore(outcome: &RestoreOutcome) -> (bool, String) {
    match outcome {
        RestoreOutcome::Restored {
            id,
            path,
            removed_checkpoint,
        } => {
            let mut text = if *removed_checkpoint {
                format!("Restored {path} to its state before the edit (checkpoint {id} removed).")
            } else {
                format!(
                    "Restored {path} to its state before the edit, but checkpoint {id} \
                     could not be removed."
                )
            };
            if path.contains('/') {
                text.push_str(" The parent directory was not removed.");
            }
            (true, text)
        }
        RestoreOutcome::Refused(refusal) => (false, restore_refusal_text(refusal)),
    }
}

/// Renders a diff outcome as (ok, message). A preview that answered —
/// even "too large" or "binary" — is a successful preview; only refused
/// gathers report not-ok.
pub(crate) fn render_diff_outcome(outcome: &DiffOutcome) -> (bool, String) {
    match outcome {
        DiffOutcome::Diff { text, .. } => (true, text.clone()),
        DiffOutcome::NothingToDiff => (
            true,
            "No difference: the file already matches its recorded before state.".to_owned(),
        ),
        DiffOutcome::BinaryOrNonText {
            before_bytes,
            current_bytes,
        } => (
            true,
            format!(
                "Not diffable: the file is binary or non-text \
                 (before {before_bytes} B, current {current_bytes} B)."
            ),
        ),
        DiffOutcome::TooLargeToDiff {
            before_bytes,
            current_bytes,
        } => (
            true,
            format!(
                "Too large to diff (before {before_bytes} B, current {current_bytes} B); \
                 the preview is limited to 64 KiB per side."
            ),
        ),
        DiffOutcome::Refused(refusal) => (false, restore_refusal_text(refusal)),
    }
}
