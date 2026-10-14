//! W13 recovery: black-box coverage for the `recover_apply_back` refusals and
//! the outcome counters its existing W11b tests do not reach.
#![cfg(any(target_os = "macos", target_os = "linux"))]

#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)]
#[path = "candidate_integration/apply_support.rs"]
mod apply_support;
#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)]
#[path = "managed_worktree_creation/create_support.rs"]
mod create_support;
#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)]
#[path = "worktree_inventory_support.rs"]
mod support;
#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)]
#[path = "guarded_applyback/support.rs"]
mod support_w11;

use saya_agent::CancellationToken;
use saya_harness::coding::integration::{
    AcceptedDelta, ApplyBackBounds, ApplyBackError, ApplyBackInput, JournalError, PlanIndex,
    RecoverError, RecoveryMode, RecoveryOutcome, apply_back, recover_apply_back,
};
use saya_types::AgentId;
use saya_types::CheckoutOperationState::{self, Completed, Failed, Intent};
use saya_types::JobId;
use std::{fs, os::unix::fs::PermissionsExt, path::Path, time::Duration};

use apply_support::{Trio, accepted_for, assigned, trio};
use support_w11::{
    apply_input, child_delta, child_write, journal_dir, journal_event_count, plan_index, state_dir,
};

/// Recovery with the shared 30-second deadline.
async fn recover(
    input: &ApplyBackInput<'_>,
    token: &CancellationToken,
    mode: RecoveryMode,
) -> Result<RecoveryOutcome, RecoverError> {
    let timeout = Duration::from_secs(30);
    recover_apply_back(input, token, timeout, mode).await
}

/// The candidate's recorded latest-operation state.
async fn recorded(t: &Trio) -> CheckoutOperationState {
    let record = t
        .fixture
        .record(t.candidate.verified.checkout_id())
        .await
        .unwrap();
    record.latest_operation.state()
}

/// The bounded apply that stops after one applied file: one Done, one before-image, open intent.
async fn stopped_apply<'a>(
    t: &'a Trio,
    plan: &'a PlanIndex,
    state: &'a Path,
    accepted: &'a [AcceptedDelta<'a>],
) -> ApplyBackInput<'a> {
    let input = apply_input(t, plan, state, accepted);
    let token = CancellationToken::new();
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
    input
}

/// An interrupted restore carries a durable Restoring record without a Restored
/// one: Finish must refuse it and Restore must resume it.
#[tokio::test]
async fn finish_refuses_an_interrupted_restore_until_restore_resumes_it() {
    let t = trio("w13-recover-mode").await;
    fs::set_permissions(
        t.fixture.repo_root.join("a/one.txt"),
        fs::Permissions::from_mode(0o640),
    )
    .unwrap();
    child_write(&t.a, "a/one.txt", b"child applied bytes\n");
    child_write(&t.a, "tracked.txt", b"child suffix bytes\n");
    let token = CancellationToken::new();
    let (delta, receipt) = child_delta(&t.a, &assigned(&["a/one.txt", "tracked.txt"]), &token);
    let accepted = [accepted_for(&t.a, &delta, &receipt)];
    let plan = plan_index(&t);
    let state = state_dir(&t);
    let input = stopped_apply(&t, &plan, &state, &accepted).await;
    let dir = journal_dir(&state, &t.fixture.job);
    let journal = dir.join("journal.jsonl");
    // Corrupt the applied file's only before-image: the restore fails after its Restoring record.
    let mut images = fs::read_dir(dir.join("before")).unwrap();
    let image = images.next().unwrap().unwrap().path();
    fs::write(&image, b"corrupt").unwrap();
    assert!(matches!(
        recover(&input, &token, RecoveryMode::Restore).await,
        Err(RecoverError::Journal(JournalError::BeforeImageMismatch))
    ));
    assert_eq!(journal_event_count(&dir, "restoring"), 1);
    assert_eq!(journal_event_count(&dir, "restored"), 0);
    let one = t.fixture.repo_root.join("a/one.txt");
    assert_eq!(fs::read(&one).unwrap(), b"child applied bytes\n");
    let journal_bytes = fs::read(&journal).unwrap();
    assert!(matches!(
        recover(&input, &token, RecoveryMode::Finish).await,
        Err(RecoverError::RecoveryModeMismatch)
    ));
    assert_eq!(journal_bytes, fs::read(&journal).unwrap());
    assert_eq!(fs::read(&one).unwrap(), b"child applied bytes\n");
    fs::write(&image, b"nested one bytes\n").unwrap();
    let outcome = recover(&input, &token, RecoveryMode::Restore)
        .await
        .unwrap();
    assert_eq!(outcome.restored(), 1);
    assert_eq!(outcome.conflicts(), 0);
    assert_eq!(outcome.finished(), 0);
    assert_eq!(outcome.already_applied(), 0);
    assert_eq!(fs::read(&one).unwrap(), b"nested one bytes\n");
    assert_eq!(
        fs::metadata(&one).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert_eq!(
        fs::read(t.fixture.repo_root.join("tracked.txt")).unwrap(),
        b"modified tracked bytes\n"
    );
    assert_eq!(journal_event_count(&dir, "restoring"), 1);
    assert_eq!(journal_event_count(&dir, "restored"), 1);
    assert_eq!(recorded(&t).await, Failed);
}

/// Refusals — foreign agent, foreign job, incomputable deadline, cancelled token —
/// happen before any effect, and the owning caller then finishes with both counters exact.
#[tokio::test]
async fn recovery_refusals_write_nothing_and_the_owner_finishes() {
    let t = trio("w13-recover-refuse").await;
    child_write(&t.a, "a/one.txt", b"child applied bytes\n");
    child_write(&t.a, "tracked.txt", b"child suffix bytes\n");
    let token = CancellationToken::new();
    let (delta, receipt) = child_delta(&t.a, &assigned(&["a/one.txt", "tracked.txt"]), &token);
    let accepted = [accepted_for(&t.a, &delta, &receipt)];
    let plan = plan_index(&t);
    let state = state_dir(&t);
    let input = stopped_apply(&t, &plan, &state, &accepted).await;
    let journal = journal_dir(&state, &t.fixture.job).join("journal.jsonl");
    let journal_bytes = fs::read(&journal).unwrap();
    let agent = AgentId::parse("agent-elsewhere").unwrap();
    let foreign_agent = ApplyBackInput {
        agent_id: Some(&agent),
        ..apply_input(&t, &plan, &state, &accepted)
    };
    let other_job = JobId::parse("job-elsewhere").unwrap();
    let foreign_job = ApplyBackInput {
        job_id: &other_job,
        ..apply_input(&t, &plan, &state, &accepted)
    };
    assert!(matches!(
        recover(&foreign_agent, &token, RecoveryMode::Finish).await,
        Err(RecoverError::NoOpenApplyBack)
    ));
    assert!(matches!(
        recover(&foreign_job, &token, RecoveryMode::Finish).await,
        Err(RecoverError::NoOpenApplyBack)
    ));
    assert!(matches!(
        recover_apply_back(&input, &token, Duration::MAX, RecoveryMode::Finish).await,
        Err(RecoverError::DeadlineExceeded)
    ));
    token.cancel();
    assert!(matches!(
        recover(&input, &token, RecoveryMode::Finish).await,
        Err(RecoverError::Cancelled)
    ));
    assert_eq!(journal_bytes, fs::read(&journal).unwrap());
    assert_eq!(
        fs::read(t.fixture.repo_root.join("a/one.txt")).unwrap(),
        b"child applied bytes\n"
    );
    assert_eq!(
        fs::read(t.fixture.repo_root.join("tracked.txt")).unwrap(),
        b"modified tracked bytes\n"
    );
    assert_eq!(recorded(&t).await, Intent);
    let live = CancellationToken::new();
    let outcome = recover(&input, &live, RecoveryMode::Finish).await.unwrap();
    assert_eq!(outcome.already_applied(), 1);
    assert_eq!(outcome.finished(), 1);
    assert_eq!(outcome.conflicts(), 0);
    assert_eq!(outcome.restored(), 0);
    assert_eq!(
        fs::read(t.fixture.repo_root.join("tracked.txt")).unwrap(),
        b"child suffix bytes\n"
    );
    assert_eq!(recorded(&t).await, Completed);
}

/// A user revert of an applied file back to its baseline bytes after a crash is
/// reported as a conflict and never re-applied; without the revert the run finishes cleanly.
#[tokio::test]
async fn a_reverted_applied_file_is_reported_and_never_reapplied() {
    for reverted in [false, true] {
        let label = ["w13-recover-clean", "w13-recover-revert"][usize::from(reverted)];
        let t = trio(label).await;
        child_write(&t.a, "a/one.txt", b"child applied bytes\n");
        child_write(&t.a, "tracked.txt", b"child suffix bytes\n");
        let token = CancellationToken::new();
        let (delta, receipt) = child_delta(&t.a, &assigned(&["a/one.txt", "tracked.txt"]), &token);
        let accepted = [accepted_for(&t.a, &delta, &receipt)];
        let plan = plan_index(&t);
        let state = state_dir(&t);
        let input = stopped_apply(&t, &plan, &state, &accepted).await;
        let one = t.fixture.repo_root.join("a/one.txt");
        if reverted {
            fs::write(&one, b"nested one bytes\n").unwrap();
        }
        let mode_before = fs::metadata(&one).unwrap().permissions().mode();
        let outcome = recover(&input, &token, RecoveryMode::Finish).await.unwrap();
        if reverted {
            assert_eq!(outcome.conflicts(), 1);
            assert_eq!(outcome.finished(), 1);
            assert_eq!(outcome.already_applied(), 0);
            assert_eq!(fs::read(&one).unwrap(), b"nested one bytes\n");
            assert_eq!(recorded(&t).await, Failed);
        } else {
            assert_eq!(outcome.already_applied(), 1);
            assert_eq!(outcome.finished(), 1);
            assert_eq!(outcome.conflicts(), 0);
            assert_eq!(fs::read(&one).unwrap(), b"child applied bytes\n");
            assert_eq!(recorded(&t).await, Completed);
        }
        let mode_after = fs::metadata(&one).unwrap().permissions().mode();
        assert_eq!(mode_after, mode_before);
        assert_eq!(
            fs::read(t.fixture.repo_root.join("tracked.txt")).unwrap(),
            b"child suffix bytes\n"
        );
    }
}
