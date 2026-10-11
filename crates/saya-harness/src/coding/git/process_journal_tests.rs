use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

#[test]
fn zero_byte_marker_failure_keeps_prior_session_unmatched() {
    let root = temp_root();
    let path = root.join(JOURNAL_NAME);
    std::fs::write(&path, wire::encode_session_start("prior").unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let before = std::fs::read(&path).unwrap();
    let mut file = OpenOptions::new().read(true).open(&path).unwrap();
    let mut appendable = true;
    assert!(matches!(
        write_incomplete_marker(&mut file, before.len(), &mut appendable),
        Err(JournalAppendError::Io)
    ));
    assert!(!appendable);
    drop(file);
    assert_eq!(std::fs::read(&path).unwrap(), before);

    let (journal, state) = GitProcessJournal::open(&root, "current").unwrap();
    assert!(state.incomplete);
    assert!(!state.session_started);
    assert!(matches!(
        journal.append(&dummy_receipt("current", 1)),
        Err(JournalAppendError::AlreadyIncomplete)
    ));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn clean_session_reopens_and_starts_a_new_identity() {
    let root = temp_root();
    let (first, state) = GitProcessJournal::open(&root, "first").unwrap();
    assert!(!state.incomplete);
    assert!(state.session_started);
    first.append(&dummy_receipt("first", 1)).unwrap();
    first.close_session().unwrap();
    drop(first);

    let (second, state) = GitProcessJournal::open(&root, "second").unwrap();
    assert!(!state.incomplete);
    assert!(state.session_started);
    second.append(&dummy_receipt("second", 1)).unwrap();
    second.close_session().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn session_close_without_start_is_incomplete() {
    let root = temp_root();
    let path = root.join(JOURNAL_NAME);
    std::fs::write(&path, wire::encode_session_close("orphan").unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let (_, state) = GitProcessJournal::open(&root, "current").unwrap();
    assert!(state.incomplete);
    assert!(!state.session_started);
    std::fs::remove_dir_all(root).unwrap();
}

fn temp_root() -> std::path::PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let root = std::env::temp_dir().join(format!(
        "saya-git-journal-session-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&root).unwrap();
    root
}

fn dummy_receipt(identity: &str, sequence: u64) -> GitProcessReceipt {
    GitProcessReceipt {
        version: 1,
        service_identity: identity.into(),
        sequence,
        request_kind: "version",
        operation: super::super::command::GitOperation::Read,
        pid: Some(1),
        exit_code: Some(0),
        signal: None,
        outcome: super::super::process_observer::GitProcessOutcome::Exited,
        effect: super::super::process_observer::GitProcessEffect::None,
        killed_orphans: None,
        duration_ms: 1,
        stdout_bytes: 0,
        stderr_bytes: 0,
        stdout_truncated: false,
        stderr_truncated: false,
    }
}
