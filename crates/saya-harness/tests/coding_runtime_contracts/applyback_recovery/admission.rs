//! Whole-operation exclusion and exact original permissions are apply-back contracts.

use super::apply_support::{accepted_for, assigned, trio};
use super::support_w11::{apply_input, child_delta, child_write, plan_index, state_dir};
use saya_agent::CancellationToken;
use saya_harness::coding::integration::{
    ApplyBackBounds, ApplyBackError, RecoveryMode, apply_back, recover_apply_back,
};
use std::{fs, os::unix::fs::PermissionsExt, time::Duration};

#[tokio::test]
async fn apply_and_restore_preserve_private_original_permissions() {
    let t = trio("recover-private").await;
    let path = t.fixture.repo_root.join("a/one.txt");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    child_write(&t.a, "a/one.txt", b"prefix\n");
    fs::set_permissions(
        t.a.root.join("a/one.txt"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
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
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let outcome = recover_apply_back(
        &input,
        &token,
        Duration::from_secs(30),
        RecoveryMode::Restore,
    )
    .await
    .unwrap();
    assert_eq!(outcome.restored(), 1);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(fs::read(path).unwrap(), b"nested one bytes\n");
}
