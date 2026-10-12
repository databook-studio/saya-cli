//! Deterministic owner-boundary checks for the actual kernel lease and journal append cap.

use super::{
    recover_binding::MAX_RECOVERY_JOURNAL_BYTES,
    transaction::ApplyBackError,
    transaction_journal::{ApplyBackJournal, JournalError, JournalEvent},
    transaction_lease::acquire,
};
use crate::workspace::Workspace;
use saya_types::JobId;
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

struct Temp(PathBuf);
impl Temp {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("saya-{label}-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn original_tree_lease_is_exclusive_and_released_on_drop() {
    let temp = Temp::new("apply-lease");
    let original = temp.0.join("original");
    fs::create_dir(&original).unwrap();
    let workspace = Workspace::open(&original).unwrap();
    let first = acquire(&temp.0, &workspace).unwrap();
    assert!(matches!(
        acquire(&temp.0, &workspace),
        Err(ApplyBackError::OperationBusy)
    ));
    drop(first);
    let second = acquire(&temp.0, &workspace).unwrap();
    drop(second);
    assert!(acquire(&temp.0, &workspace).is_ok());
}

#[test]
fn a_lease_released_just_after_the_attempt_started_is_still_acquired() {
    // A child forked by another thread holds an inherited copy of the lease
    // descriptor until it execs, so the kernel releases the lock a moment
    // after the owner dropped it. The next acquire must ride that out.
    let temp = Temp::new("apply-lease-handoff");
    let original = temp.0.join("original");
    fs::create_dir(&original).unwrap();
    let workspace = Workspace::open(&original).unwrap();
    let held = acquire(&temp.0, &workspace).unwrap();
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(30));
        drop(held);
    });
    assert!(acquire(&temp.0, &workspace).is_ok());
    releaser.join().unwrap();
}

#[test]
fn over_budget_journal_append_leaves_the_existing_prefix_untouched() {
    let temp = Temp::new("apply-journal-cap");
    let mut journal = ApplyBackJournal::create(
        &temp.0,
        &JobId::parse("journal-cap").unwrap(),
        std::num::NonZeroU64::MIN,
    )
    .unwrap();
    journal.file.set_len(MAX_RECOVERY_JOURNAL_BYTES).unwrap();
    let event = JournalEvent::Done {
        path: "owned.txt".into(),
        result: None,
    };
    assert!(matches!(
        journal.append(&event),
        Err(JournalError::OverBudget)
    ));
    assert_eq!(
        journal.file.metadata().unwrap().len(),
        MAX_RECOVERY_JOURNAL_BYTES
    );
}
