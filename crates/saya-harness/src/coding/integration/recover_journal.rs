//! Bound, source-checked replay of the per-file journal; persisted paths grant no authority.

use super::{
    DeltaKind, PathDelta,
    transaction::ApplyBackInput,
    transaction_journal::{
        ApplyBackJournal, BeforeImageStore, JournalError, JournalEvent, JournalKind, sha_hex,
    },
};
use crate::workspace::{MAX_IO_BYTES, Workspace};
use saya_types::Sha256Digest;
use std::{
    collections::{HashMap, HashSet},
    num::NonZeroU64,
};

pub(super) use super::recover_binding::{MAX_RECOVERY_JOURNAL_BYTES, operation_binding};

pub(super) fn intent_event(
    input: &ApplyBackInput<'_>,
    entry: &PathDelta,
    image: Option<String>,
) -> JournalEvent {
    let baseline = input
        .baseline
        .entries()
        .iter()
        .find(|base| base.path().as_str() == entry.path());
    JournalEvent::Intent {
        path: entry.path().to_owned(),
        kind: match entry.kind() {
            DeltaKind::Added => JournalKind::Added,
            DeltaKind::Modified => JournalKind::Modified,
            DeltaKind::Deleted => JournalKind::Deleted,
        },
        baseline: baseline.map(|base| base.content_sha256().as_str().to_owned()),
        delta: entry.content_sha256().map(|sha| sha.as_str().to_owned()),
        image,
    }
}

pub(super) struct RecoveryJournal {
    journal: ApplyBackJournal,
    intents: HashMap<String, JournalEvent>,
    done: HashSet<String>,
    target_modes: std::collections::BTreeMap<String, u32>,
    baseline_modes: std::collections::BTreeMap<String, u32>,
    restoring: HashSet<String>,
    restored: HashSet<String>,
}

impl RecoveryJournal {
    pub(super) fn open(
        input: &ApplyBackInput<'_>,
        sequence: NonZeroU64,
        original: &Workspace,
    ) -> Result<Self, JournalError> {
        let (mut journal, events, valid) =
            ApplyBackJournal::reopen(input.state_dir, input.job_id, sequence)?;
        let Some(JournalEvent::Begin {
            binding: recorded,
            baseline_modes,
        }) = events.first()
        else {
            return Err(JournalError::Corrupt);
        };
        let binding = operation_binding(input, sequence, original, Some(baseline_modes))?;
        if recorded != &binding.digest {
            return Err(JournalError::Corrupt);
        }
        let expected: HashMap<&str, &PathDelta> = input
            .accepted
            .iter()
            .flat_map(|accepted| accepted.delta().entries())
            .map(|entry| (entry.path(), entry))
            .collect();
        let mut intents = HashMap::new();
        let mut done = HashSet::new();
        let mut restoring = HashSet::new();
        let mut restored = HashSet::new();
        for event in events.into_iter().skip(1) {
            match &event {
                JournalEvent::Intent { path, image, .. } => {
                    let entry = expected.get(path.as_str()).ok_or(JournalError::Corrupt)?;
                    let expected_image = if entry.kind() == DeltaKind::Added {
                        None
                    } else {
                        input
                            .baseline
                            .entries()
                            .iter()
                            .find(|base| base.path().as_str() == entry.path())
                            .map(|base| base.content_sha256().as_str().to_owned())
                    };
                    if *image != expected_image
                        || event != intent_event(input, entry, expected_image)
                        || intents.contains_key(path)
                    {
                        return Err(JournalError::Corrupt);
                    }
                    intents.insert(path.clone(), event);
                }
                JournalEvent::Done { path, result } => {
                    let entry = expected.get(path.as_str()).ok_or(JournalError::Corrupt)?;
                    if !intents.contains_key(path)
                        || !done.insert(path.clone())
                        || result.as_deref() != entry.content_sha256().map(|sha| sha.as_str())
                    {
                        return Err(JournalError::Corrupt);
                    }
                }
                JournalEvent::Begin { .. } => return Err(JournalError::Corrupt),
                JournalEvent::Restoring { path } => {
                    if !intents.contains_key(path) || !restoring.insert(path.clone()) {
                        return Err(JournalError::Corrupt);
                    }
                }
                JournalEvent::Restored { path } => {
                    if !restoring.contains(path) || !restored.insert(path.clone()) {
                        return Err(JournalError::Corrupt);
                    }
                }
            }
        }
        journal.truncate_tail(valid)?;
        Ok(Self {
            journal,
            intents,
            done,
            target_modes: binding.target_modes,
            baseline_modes: binding.baseline_modes,
            restoring,
            restored,
        })
    }

    pub(super) fn has_intent(&self, path: &str) -> bool {
        self.intents.contains_key(path)
    }

    pub(super) fn target_mode(&self, path: &str) -> Option<u32> {
        self.target_modes.get(path).copied()
    }

    pub(super) fn baseline_mode(&self, path: &str) -> Option<u32> {
        self.baseline_modes.get(path).copied()
    }
    pub(super) fn is_done(&self, path: &str) -> bool {
        self.done.contains(path)
    }

    pub(super) fn restoring(&self, path: &str) -> bool {
        self.restoring.contains(path) && !self.restored.contains(path)
    }
    pub(super) fn was_restored(&self, path: &str) -> bool {
        self.restored.contains(path)
    }

    pub(super) fn has_restores(&self) -> bool {
        !self.restoring.is_empty()
    }

    pub(super) fn begin_restore(&mut self, path: &str) -> Result<(), JournalError> {
        if !self.restoring.contains(path) {
            self.journal.append(&JournalEvent::Restoring {
                path: path.to_owned(),
            })?;
            self.restoring.insert(path.to_owned());
        }
        Ok(())
    }

    pub(super) fn finish_restore(&mut self, path: &str) -> Result<(), JournalError> {
        if !self.restored.contains(path) {
            self.journal.append(&JournalEvent::Restored {
                path: path.to_owned(),
            })?;
            self.restored.insert(path.to_owned());
        }
        Ok(())
    }

    pub(super) fn intent(
        &mut self,
        input: &ApplyBackInput<'_>,
        original: &Workspace,
        entry: &PathDelta,
    ) -> Result<(), JournalError> {
        if self.has_intent(entry.path()) {
            return Ok(());
        }
        let image = if entry.kind() == DeltaKind::Added {
            None
        } else {
            let base = input
                .baseline
                .entries()
                .iter()
                .find(|base| base.path().as_str() == entry.path())
                .ok_or(JournalError::Corrupt)?;
            let store = Workspace::open(&self.journal.dir().join("before"))
                .map_err(|_| JournalError::Io)?;
            let name = sha_hex(entry.path().as_bytes());
            // A prior crash after image capture but before intent may leave this image.
            match store.read(&name, MAX_IO_BYTES as u64) {
                Ok(read) if read.digest == base.content_sha256().as_str() && !read.truncated => {
                    Some(read.digest)
                }
                Err(error) if error.is_not_found() => {
                    Some(BeforeImageStore::new(self.journal.dir()).capture(
                        original,
                        entry.path(),
                        base.content_sha256(),
                    )?)
                }
                _ => return Err(JournalError::BeforeImageMismatch),
            }
        };
        let event = intent_event(input, entry, image);
        self.journal.append(&event)?;
        self.intents.insert(entry.path().to_owned(), event);
        Ok(())
    }

    pub(super) fn done(&mut self, entry: &PathDelta) -> Result<(), JournalError> {
        if self.done.contains(entry.path()) {
            return Ok(());
        }
        self.journal.append(&JournalEvent::Done {
            path: entry.path().to_owned(),
            result: entry.content_sha256().map(|sha| sha.as_str().to_owned()),
        })?;
        self.done.insert(entry.path().to_owned());
        Ok(())
    }

    pub(super) fn before_bytes(
        &self,
        path: &str,
        expected: &Sha256Digest,
    ) -> Result<Vec<u8>, JournalError> {
        BeforeImageStore::new(self.journal.dir()).load(path, expected)
    }
}
