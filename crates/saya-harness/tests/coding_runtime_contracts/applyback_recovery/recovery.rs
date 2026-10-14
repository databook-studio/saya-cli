//! Recovery preserves user drift and reconciles the complete accepted change set.

use saya_agent::CancellationToken;
use saya_harness::coding::integration::{
    ApplyBackBounds, ApplyBackError, RecoverError, RecoveryMode, apply_back, recover_apply_back,
};
use saya_types::{CheckoutOperationState, ManagedCheckoutId};
use std::{fs, os::unix::fs::PermissionsExt, time::Duration};

use super::apply_support::{accepted_for, assigned, trio};
use super::support_w11::{
    apply_input, child_delta, child_write, journal_dir, plan_index, state_dir,
};

#[tokio::test]
async fn recovery_preserves_new_user_edit_and_retains_evidence() {
    let t = trio("recover-user").await;
    child_write(&t.a, "a/one.txt", b"child first\n");
    child_write(&t.a, "tracked.txt", b"child second\n");
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
    fs::write(t.fixture.repo_root.join("tracked.txt"), b"new user bytes\n").unwrap();
    let outcome = recover_apply_back(
        &input,
        &token,
        Duration::from_secs(30),
        RecoveryMode::Finish,
    )
    .await
    .unwrap();
    assert_eq!(outcome.conflicts(), 1);
    assert_eq!(
        fs::read(t.fixture.repo_root.join("tracked.txt")).unwrap(),
        b"new user bytes\n"
    );
    assert_eq!(
        fs::read(t.fixture.repo_root.join("a/one.txt")).unwrap(),
        b"child first\n"
    );
    assert!(journal_dir(&state, &t.fixture.job).join("before").exists());
    let record = t
        .fixture
        .record(&ManagedCheckoutId::parse("recover-user-c").unwrap())
        .await
        .unwrap();
    assert_eq!(
        record.latest_operation.state(),
        CheckoutOperationState::Failed
    );
}

#[tokio::test]
async fn recovery_finishes_unjournaled_suffix_repairs_mode_and_closes_once() {
    let t = trio("recover-finish").await;
    fs::set_permissions(
        t.fixture.repo_root.join("a/one.txt"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    child_write(&t.a, "a/one.txt", b"child first\n");
    fs::set_permissions(
        t.a.root.join("a/one.txt"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    child_write(&t.a, "tracked.txt", b"child suffix\n");
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
    // Remove the final durable Done to reproduce the earlier publication window.
    let journal_path = journal_dir(&state, &t.fixture.job).join("journal.jsonl");
    let journal_bytes = fs::read(&journal_path).unwrap();
    let previous_end = journal_bytes[..journal_bytes.len() - 1]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .unwrap()
        + 1;
    fs::OpenOptions::new()
        .write(true)
        .open(&journal_path)
        .unwrap()
        .set_len(previous_end as u64)
        .unwrap();
    // Reproduce the observable bytes-published/mode-not-published crash state.
    fs::set_permissions(
        t.fixture.repo_root.join("a/one.txt"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    // A torn multibyte final line must not hide the preceding durable prefix.
    use std::io::Write;
    fs::OpenOptions::new()
        .append(true)
        .open(journal_dir(&state, &t.fixture.job).join("journal.jsonl"))
        .unwrap()
        .write_all(b"{\"path\":\"\xc3")
        .unwrap();
    let outcome = recover_apply_back(
        &input,
        &token,
        Duration::from_secs(30),
        RecoveryMode::Finish,
    )
    .await
    .unwrap();
    assert_eq!(outcome.conflicts(), 0);
    assert_eq!(outcome.finished(), 1);
    assert_eq!(
        fs::read(t.fixture.repo_root.join("tracked.txt")).unwrap(),
        b"child suffix\n"
    );
    assert_eq!(
        fs::metadata(t.fixture.repo_root.join("a/one.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    let record = t
        .fixture
        .record(t.candidate.verified.checkout_id())
        .await
        .unwrap();
    assert_eq!(
        record.latest_operation.state(),
        CheckoutOperationState::Completed
    );
    assert!(matches!(
        recover_apply_back(
            &input,
            &token,
            Duration::from_secs(30),
            RecoveryMode::Finish
        )
        .await,
        Err(RecoverError::NoOpenApplyBack)
    ));
    assert_eq!(
        record,
        t.fixture
            .record(t.candidate.verified.checkout_id())
            .await
            .unwrap()
    );
    let journal = fs::read(journal_dir(&state, &t.fixture.job).join("journal.jsonl")).unwrap();
    assert!(std::str::from_utf8(&journal).is_ok());
}

#[tokio::test]
async fn restore_replaces_only_the_recorded_applied_state() {
    for user_edit in [false, true] {
        let t = trio(if user_edit {
            "recover-restore-user"
        } else {
            "recover-restore"
        })
        .await;
        child_write(&t.a, "a/one.txt", b"applied bytes\n");
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
        if user_edit {
            fs::write(
                t.fixture.repo_root.join("a/one.txt"),
                b"user changed applied\n",
            )
            .unwrap();
        }
        let outcome = recover_apply_back(
            &input,
            &token,
            Duration::from_secs(30),
            RecoveryMode::Restore,
        )
        .await
        .unwrap();
        assert_eq!(outcome.conflicts(), usize::from(user_edit));
        assert_eq!(outcome.restored(), usize::from(!user_edit));
        let expected: &[u8] = if user_edit {
            b"user changed applied\n"
        } else {
            b"nested one bytes\n"
        };
        assert_eq!(
            fs::read(t.fixture.repo_root.join("a/one.txt")).unwrap(),
            expected
        );
        assert_eq!(
            fs::read(t.fixture.repo_root.join("tracked.txt")).unwrap(),
            b"modified tracked bytes\n"
        );
        let record = t
            .fixture
            .record(t.candidate.verified.checkout_id())
            .await
            .unwrap();
        assert_eq!(
            record.latest_operation.state(),
            CheckoutOperationState::Failed
        );
    }
}
