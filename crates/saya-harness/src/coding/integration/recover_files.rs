//! Per-file reconciliation uses trusted deltas and fresh guarded filesystem operations.

use super::{
    ApplyError, DeltaKind, PathDelta,
    apply::apply_entry_with_mode,
    apply_fs::{file_mode, read_child_file, set_candidate_mode_bits, sync_applied_entry},
    recover::{RecoverError, RecoveryMode, RecoveryOutcome},
    recover_journal::RecoveryJournal,
    transaction::{ApplyBackError, ApplyBackInput},
};
use crate::{HarnessError, workspace::Workspace};

enum LiveState {
    Missing,
    File(String),
    Unsafe,
}

fn live_state(workspace: &Workspace, path: &str) -> Result<LiveState, ApplyBackError> {
    match workspace.read(path, 0) {
        Ok(read) => Ok(LiveState::File(read.digest)),
        Err(error) if error.is_not_found() => Ok(LiveState::Missing),
        Err(
            HarnessError::SymlinkRefused { .. }
            | HarnessError::NotRegularFile { .. }
            | HarnessError::IdentityChanged { .. }
            | HarnessError::BoundsExceeded { .. },
        ) => Ok(LiveState::Unsafe),
        Err(_) => Err(ApplyError::Fs.into()),
    }
}

pub(super) fn reconcile_file(
    input: &ApplyBackInput<'_>,
    original: &Workspace,
    child: &Workspace,
    entry: &PathDelta,
    mode: RecoveryMode,
    journal: &mut RecoveryJournal,
    outcome: &mut RecoveryOutcome,
) -> Result<(), RecoverError> {
    let actual = live_state(original, entry.path())?;
    let baseline = input
        .baseline
        .entries()
        .iter()
        .find(|base| base.path().as_str() == entry.path());
    let actual_digest = match &actual {
        LiveState::File(digest) => Some(digest.as_str()),
        _ => None,
    };
    let has_intent = journal.has_intent(entry.path());
    let observed_mode = if matches!(actual, LiveState::File(_)) {
        Some(file_mode(original, entry.path()).map_err(ApplyBackError::from)?)
    } else {
        None
    };
    let restored_bytes = mode == RecoveryMode::Restore
        && baseline.is_some_and(|base| Some(base.content_sha256().as_str()) == actual_digest);
    let transient = has_intent
        && observed_mode == Some(0o600)
        && (!journal.is_done(entry.path()) || restored_bytes && journal.restoring(entry.path()));
    let at_baseline = match (&actual, baseline) {
        (LiveState::Missing, None) => true,
        (LiveState::File(digest), Some(base)) => {
            let expected_mode = journal
                .baseline_mode(entry.path())
                .ok_or(super::JournalError::Corrupt)?;
            digest == base.content_sha256().as_str()
                && (observed_mode == Some(expected_mode) || transient)
        }
        _ => false,
    };
    let at_applied = has_intent
        && match entry.kind() {
            DeltaKind::Deleted => matches!(actual, LiveState::Missing),
            _ => {
                let expected_mode = journal
                    .target_mode(entry.path())
                    .ok_or(super::JournalError::Corrupt)?;
                actual_digest == entry.content_sha256().map(|sha| sha.as_str())
                    && (observed_mode == Some(expected_mode) || transient)
            }
        };
    if journal.was_restored(entry.path()) && !at_baseline {
        outcome.conflicts += 1;
        return Ok(());
    }
    if at_baseline
        && !at_applied
        && journal.is_done(entry.path())
        && mode == RecoveryMode::Finish
        && !journal.was_restored(entry.path())
    {
        outcome.conflicts += 1;
        return Ok(());
    }
    // Without our durable intent, even identical user-added bytes are not ours.
    if at_applied {
        if mode == RecoveryMode::Finish {
            if entry.kind() != DeltaKind::Deleted {
                let file = read_child_file(child, entry.path()).map_err(ApplyBackError::from)?;
                if Some(file.sha256_hex.as_str()) != entry.content_sha256().map(|sha| sha.as_str())
                    || Some(file.executable)
                        != journal
                            .target_mode(entry.path())
                            .map(|mode| mode & 0o111 != 0)
                {
                    return Err(ApplyBackError::Apply(ApplyError::ContentMismatch).into());
                }
                // A fresh guarded rewrite keeps a concurrent user edit from being
                // silently chmod'ed after the earlier digest observation.
                original
                    .write_guarded(entry.path(), &file.sha256_hex, &file.bytes)
                    .map_err(|_| ApplyBackError::Apply(ApplyError::Fs))?;
                set_candidate_mode_bits(
                    original,
                    entry.path(),
                    journal
                        .target_mode(entry.path())
                        .ok_or(super::JournalError::Corrupt)?,
                )
                .map_err(ApplyBackError::from)?;
            }
            sync_applied_entry(original, entry.path(), entry.kind() == DeltaKind::Deleted)
                .map_err(ApplyBackError::from)?;
            journal.done(entry)?;
            outcome.already_applied += 1;
        } else {
            journal.begin_restore(entry.path())?;
            restore(input, original, entry, journal)?;
            journal.finish_restore(entry.path())?;
            outcome.restored += 1;
        }
    } else if at_baseline {
        if mode == RecoveryMode::Finish {
            journal.intent(input, original, entry)?;
            apply_entry_with_mode(
                original,
                child,
                entry,
                input.baseline,
                journal.target_mode(entry.path()),
            )
            .map_err(ApplyBackError::from)?;
            sync_applied_entry(original, entry.path(), entry.kind() == DeltaKind::Deleted)
                .map_err(ApplyBackError::from)?;
            journal.done(entry)?;
            outcome.finished += 1;
        } else if let Some(base) = baseline
            && journal.restoring(entry.path())
        {
            // Repair a restore interrupted between publishing bytes and mode.
            let read = original
                .read(entry.path(), crate::workspace::MAX_IO_BYTES as u64)
                .map_err(|_| ApplyBackError::Apply(ApplyError::Fs))?;
            original
                .write_guarded(entry.path(), base.content_sha256().as_str(), &read.bytes)
                .map_err(|_| ApplyBackError::Apply(ApplyError::Fs))?;
            set_candidate_mode_bits(
                original,
                entry.path(),
                journal
                    .baseline_mode(entry.path())
                    .ok_or(super::JournalError::Corrupt)?,
            )
            .map_err(ApplyBackError::from)?;
            sync_applied_entry(original, entry.path(), false).map_err(ApplyBackError::from)?;
            journal.finish_restore(entry.path())?;
        } else if baseline.is_none() && journal.restoring(entry.path()) {
            sync_applied_entry(original, entry.path(), true).map_err(ApplyBackError::from)?;
            journal.finish_restore(entry.path())?;
        }
    } else {
        outcome.conflicts += 1;
    }
    Ok(())
}

fn restore(
    input: &ApplyBackInput<'_>,
    original: &Workspace,
    entry: &PathDelta,
    journal: &RecoveryJournal,
) -> Result<(), RecoverError> {
    if entry.kind() == DeltaKind::Added {
        let digest = entry
            .content_sha256()
            .ok_or(ApplyBackError::Apply(ApplyError::ContentMismatch))?;
        original
            .remove_guarded(entry.path(), digest.as_str())
            .map_err(|_| ApplyBackError::Apply(ApplyError::Fs))?;
        sync_applied_entry(original, entry.path(), true).map_err(ApplyBackError::from)?;
        return Ok(());
    }
    let base = input
        .baseline
        .entries()
        .iter()
        .find(|base| base.path().as_str() == entry.path())
        .ok_or(ApplyBackError::Apply(ApplyError::ContentMismatch))?;
    let bytes = journal.before_bytes(entry.path(), base.content_sha256())?;
    if entry.kind() == DeltaKind::Deleted {
        original
            .create_new(entry.path(), &bytes)
            .map_err(|_| ApplyBackError::Apply(ApplyError::Fs))?;
    } else {
        let digest = entry
            .content_sha256()
            .ok_or(ApplyBackError::Apply(ApplyError::ContentMismatch))?;
        original
            .write_guarded(entry.path(), digest.as_str(), &bytes)
            .map_err(|_| ApplyBackError::Apply(ApplyError::Fs))?;
    }
    set_candidate_mode_bits(
        original,
        entry.path(),
        journal
            .baseline_mode(entry.path())
            .ok_or(super::JournalError::Corrupt)?,
    )
    .map_err(ApplyBackError::from)?;
    sync_applied_entry(original, entry.path(), false).map_err(ApplyBackError::from)?;
    Ok(())
}
