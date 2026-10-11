//! The durable source binding prevents replacing an interrupted operation's suffix.

use super::apply_support::{accepted_for, assigned, trio};
use super::support_w11::{
    apply_input, child_delta, child_write, journal_dir, plan_index, state_dir,
};
use saya_agent::CancellationToken;
use saya_harness::coding::integration::{
    ApplyBackBounds, ApplyBackError, JournalError, RecoverError, RecoveryMode, apply_back,
    recover_apply_back,
};
use saya_types::CheckoutOperationState;
use std::{fs, time::Duration};

#[tokio::test]
async fn substituted_unjournaled_suffix_is_refused_without_effects() {
    let t = trio("recover-binding").await;
    child_write(&t.a, "a/one.txt", b"prefix\n");
    child_write(&t.a, "tracked.txt", b"accepted suffix\n");
    let token = CancellationToken::new();
    let assignment = assigned(&["a/one.txt", "tracked.txt"]);
    let (delta, receipt) = child_delta(&t.a, &assignment, &token);
    let accepted = [accepted_for(&t.a, &delta, &receipt)];
    let plan = plan_index(&t);
    let state = state_dir(&t);
    assert!(matches!(
        apply_back(
            &apply_input(&t, &plan, &state, &accepted),
            &token,
            ApplyBackBounds {
                stop_after_files: Some(1),
                ..ApplyBackBounds::default()
            }
        )
        .await,
        Err(ApplyBackError::Stopped)
    ));
    let path = journal_dir(&state, &t.fixture.job).join("journal.jsonl");
    let journal_before = fs::read(&path).unwrap();
    child_write(&t.a, "tracked.txt", b"substituted suffix\n");
    let (new_delta, new_receipt) = child_delta(&t.a, &assignment, &token);
    let replacement = [accepted_for(&t.a, &new_delta, &new_receipt)];
    let result = recover_apply_back(
        &apply_input(&t, &plan, &state, &replacement),
        &token,
        Duration::from_secs(30),
        RecoveryMode::Finish,
    )
    .await;
    assert!(matches!(
        result,
        Err(RecoverError::Journal(JournalError::Corrupt))
    ));
    assert_eq!(
        fs::read(t.fixture.repo_root.join("a/one.txt")).unwrap(),
        b"prefix\n"
    );
    assert_eq!(
        fs::read(t.fixture.repo_root.join("tracked.txt")).unwrap(),
        b"modified tracked bytes\n"
    );
    assert_eq!(journal_before, fs::read(path).unwrap());
    let record = t
        .fixture
        .record(t.candidate.verified.checkout_id())
        .await
        .unwrap();
    assert_eq!(
        record.latest_operation.state(),
        CheckoutOperationState::Intent
    );
}

#[tokio::test]
async fn restore_handles_added_and_deleted_files() {
    for deleted in [false, true] {
        let t = trio(if deleted {
            "recover-delete"
        } else {
            "recover-add"
        })
        .await;
        let path = if deleted { "a/one.txt" } else { "a/new.txt" };
        if deleted {
            fs::remove_file(t.a.root.join(path)).unwrap();
        } else {
            child_write(&t.a, path, b"new child file\n");
        }
        child_write(&t.a, "tracked.txt", b"suffix\n");
        let token = CancellationToken::new();
        let (delta, receipt) = child_delta(&t.a, &assigned(&[path, "tracked.txt"]), &token);
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
        let outcome = recover_apply_back(
            &input,
            &token,
            Duration::from_secs(30),
            RecoveryMode::Restore,
        )
        .await
        .unwrap();
        assert_eq!(outcome.restored(), 1);
        assert_eq!(outcome.conflicts(), 0);
        if deleted {
            assert_eq!(
                fs::read(t.fixture.repo_root.join(path)).unwrap(),
                b"nested one bytes\n"
            );
        } else {
            assert!(!t.fixture.repo_root.join(path).exists());
        }
        assert_eq!(
            fs::read(t.fixture.repo_root.join("tracked.txt")).unwrap(),
            b"modified tracked bytes\n"
        );
    }
}
