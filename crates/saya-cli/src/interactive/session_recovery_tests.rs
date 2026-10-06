//! The recovery handle's own contract: the enable flag is OFF at
//! construction, session-scoped (every clone — and every `parts()` clone —
//! shares one cell), and the parts hand over the same store, id, and flag.

use std::sync::Arc;

use super::RecoveryHandle;

fn temp_root(label: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("saya-recovery-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    root
}

#[test]
fn the_flag_defaults_off_and_every_clone_shares_one_cell() {
    let root = temp_root("flag");
    let handle = RecoveryHandle::new(&root, "flag-session");
    assert!(
        !handle.is_enabled(),
        "recovery is opt-in: the default is OFF"
    );
    handle.set_enabled(true);
    assert!(handle.is_enabled(), "the flag flips");
    let clone = handle.clone();
    assert!(clone.is_enabled(), "a clone sees the same flag cell");
    handle.set_enabled(false);
    assert!(!clone.is_enabled(), "and the same cell flips back");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn parts_hand_over_the_same_id_and_the_same_flag_cell() {
    let root = temp_root("parts");
    let handle = RecoveryHandle::new(&root, "parts-session");
    let (store, session_id, enabled) = handle.parts();
    assert_eq!(session_id, "parts-session");
    assert!(
        !enabled.load(std::sync::atomic::Ordering::Acquire),
        "the parts' flag starts off"
    );
    handle.set_enabled(true);
    assert!(
        enabled.load(std::sync::atomic::Ordering::Acquire),
        "the parts' flag is the handle's own cell"
    );
    let (store_again, _, flag_again) = handle.parts();
    assert!(Arc::ptr_eq(&store, &store_again), "one store per session");
    assert!(
        Arc::ptr_eq(&enabled, &flag_again),
        "one flag cell per session"
    );
    let _ = std::fs::remove_dir_all(&root);
}
