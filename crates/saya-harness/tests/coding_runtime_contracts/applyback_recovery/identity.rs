//! Recovery binds the original root and accepted modes, while preserving user chmod.

use super::apply_support::{accepted_for, assigned, trio};
use super::support_w11::{apply_input, child_delta, child_write, plan_index, state_dir};
use saya_agent::CancellationToken;
use saya_harness::coding::integration::{
    ApplyBackBounds, ApplyBackError, JournalError, RecoverError, RecoveryMode, apply_back,
    recover_apply_back,
};
use std::{fs, os::unix::fs::PermissionsExt, time::Duration};

#[tokio::test]
async fn new_user_permissions_are_preserved_during_finish_and_restore() {
    for (mode, user_mode) in [
        (RecoveryMode::Finish, 0o400),
        (RecoveryMode::Restore, 0o400),
        (RecoveryMode::Finish, 0o600),
        (RecoveryMode::Restore, 0o600),
    ] {
        let t = trio("recover-chmod").await;
        child_write(&t.a, "a/one.txt", b"applied prefix\n");
        child_write(&t.a, "tracked.txt", b"suffix\n");
        fs::set_permissions(
            t.fixture.repo_root.join("a/one.txt"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        let token = CancellationToken::new();
        let (delta, receipt) = child_delta(&t.a, &assigned(&["a/one.txt", "tracked.txt"]), &token);
        let accepted = [accepted_for(&t.a, &delta, &receipt)];
        let plan = plan_index(&t);
        let state = state_dir(&t);
        let input = apply_input(&t, &plan, &state, &accepted);
        assert!(matches!(
            apply_back(
                &input,
                &token,
                ApplyBackBounds {
                    stop_after_files: Some(1),
                    ..ApplyBackBounds::default()
                }
            )
            .await,
            Err(ApplyBackError::Stopped)
        ));
        let original = t.fixture.repo_root.join("a/one.txt");
        fs::set_permissions(&original, fs::Permissions::from_mode(user_mode)).unwrap();
        let outcome = recover_apply_back(&input, &token, Duration::from_secs(30), mode)
            .await
            .unwrap();
        assert_eq!(outcome.conflicts(), 1);
        assert_eq!(
            fs::metadata(&original).unwrap().permissions().mode() & 0o777,
            user_mode
        );
        assert_eq!(fs::read(original).unwrap(), b"applied prefix\n");
    }
}

#[tokio::test]
async fn changed_child_executable_mode_in_unjournaled_suffix_is_refused() {
    let t = trio("recover-child-mode").await;
    child_write(&t.a, "a/one.txt", b"prefix\n");
    child_write(&t.a, "tracked.txt", b"suffix\n");
    let token = CancellationToken::new();
    let (delta, receipt) = child_delta(&t.a, &assigned(&["a/one.txt", "tracked.txt"]), &token);
    let accepted = [accepted_for(&t.a, &delta, &receipt)];
    let plan = plan_index(&t);
    let state = state_dir(&t);
    let input = apply_input(&t, &plan, &state, &accepted);
    assert!(matches!(
        apply_back(
            &input,
            &token,
            ApplyBackBounds {
                stop_after_files: Some(1),
                ..ApplyBackBounds::default()
            }
        )
        .await,
        Err(ApplyBackError::Stopped)
    ));
    fs::set_permissions(
        t.a.root.join("tracked.txt"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert!(matches!(
        recover_apply_back(
            &input,
            &token,
            Duration::from_secs(30),
            RecoveryMode::Finish
        )
        .await,
        Err(RecoverError::Journal(JournalError::Corrupt))
    ));
    assert_eq!(
        fs::read(t.fixture.repo_root.join("tracked.txt")).unwrap(),
        b"modified tracked bytes\n"
    );
}

#[tokio::test]
async fn replaced_original_root_cannot_receive_an_added_suffix() {
    let t = trio("recover-root").await;
    child_write(&t.a, "a/one.txt", b"prefix\n");
    child_write(&t.a, "notes/new.txt", b"added suffix\n");
    let token = CancellationToken::new();
    let (delta, receipt) = child_delta(&t.a, &assigned(&["a/one.txt", "notes/new.txt"]), &token);
    let accepted = [accepted_for(&t.a, &delta, &receipt)];
    let plan = plan_index(&t);
    let state = state_dir(&t);
    let input = apply_input(&t, &plan, &state, &accepted);
    assert!(matches!(
        apply_back(
            &input,
            &token,
            ApplyBackBounds {
                stop_after_files: Some(1),
                ..ApplyBackBounds::default()
            }
        )
        .await,
        Err(ApplyBackError::Stopped)
    ));
    let saved = t.fixture.temp.path().join("retained-original");
    fs::rename(&t.fixture.repo_root, &saved).unwrap();
    fs::create_dir_all(t.fixture.repo_root.join("a")).unwrap();
    // Keep Git's recorded common-dir path valid so only the root inode differs.
    fs::rename(saved.join(".git"), t.fixture.repo_root.join(".git")).unwrap();
    fs::copy(
        saved.join("a/one.txt"),
        t.fixture.repo_root.join("a/one.txt"),
    )
    .unwrap();
    fs::copy(
        saved.join("tracked.txt"),
        t.fixture.repo_root.join("tracked.txt"),
    )
    .unwrap();
    assert!(matches!(
        recover_apply_back(
            &input,
            &token,
            Duration::from_secs(30),
            RecoveryMode::Finish
        )
        .await,
        Err(RecoverError::Journal(JournalError::Corrupt))
    ));
    assert!(!t.fixture.repo_root.join("notes/new.txt").exists());
    assert_eq!(fs::read(saved.join("a/one.txt")).unwrap(), b"prefix\n");
}
