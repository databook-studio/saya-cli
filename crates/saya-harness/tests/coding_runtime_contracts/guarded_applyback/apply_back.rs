//! W11 apply-back tests: journaled guarded multi-file apply-back into the
//! user's original checkout, with the intent left open on any non-success.

use saya_agent::CancellationToken;
use saya_harness::coding::integration::{
    ApplyBackBounds, ApplyBackError, DEFAULT_DELTA_DEADLINE, apply_back, apply_back_latest_dir,
};
use saya_types::{CheckoutOperationState, ManagedCheckoutId, RepositoryMutationAction};
use std::{fs, os::unix::fs::PermissionsExt};

use super::apply_support::{accepted_for, assigned, trio};
use super::support_w11::{
    apply_input, child_delta, child_remove, child_write, journal_dir, journal_event_count,
    plan_index, state_dir,
};

// W11 (6) `completed_outcome_is_recorded_once` is skipped: the single-outcome
// rule cannot be probed with public test APIs. `apply_back` records the
// `Completed` outcome itself, a re-run on a `Completed` candidate is not
// `CandidateBusy` (only `Intent`/`Uncertain` is busy), and
// `record_checkout_operation_outcome` is not callable from tests.

/// W11 (1), first failing proof: a run bounded to stop after one file leaves
/// that file's `done` line, the second file's original bytes intact, one
/// verified before-image, and an open `ApplyBack` intent in the store.
#[tokio::test]
async fn kill_mid_apply_retains_before_images() {
    let t = trio("w11-kill").await;
    child_write(&t.a, "tracked.txt", b"alpha\n");
    child_write(&t.a, "a/one.txt", b"beta\n");
    let assignment = assigned(&["a/one.txt", "tracked.txt"]);
    let token = CancellationToken::new();
    let (delta, receipt) = child_delta(&t.a, &assignment, &token);
    let accepted = accepted_for(&t.a, &delta, &receipt);
    let plan = plan_index(&t);
    let state = state_dir(&t);
    let bounds = ApplyBackBounds {
        deadline: DEFAULT_DELTA_DEADLINE,
        stop_after_files: Some(1),
    };
    let result = apply_back(&apply_input(&t, &plan, &state, &[accepted]), &token, bounds).await;
    assert!(matches!(result, Err(ApplyBackError::Stopped)));
    assert_eq!(
        fs::read_to_string(t.fixture.repo_root.join("a/one.txt")).unwrap(),
        "beta\n"
    );
    assert_eq!(
        fs::read_to_string(t.fixture.repo_root.join("tracked.txt")).unwrap(),
        "modified tracked bytes\n"
    );
    let dir = journal_dir(&state, &t.fixture.job);
    assert_eq!(journal_event_count(&dir, "done"), 1);
    assert_eq!(journal_event_count(&dir, "intent"), 1);
    let mut images = fs::read_dir(dir.join("before")).unwrap();
    let image = images.next().unwrap().unwrap();
    assert!(images.next().is_none());
    assert_eq!(fs::read(image.path()).unwrap(), b"nested one bytes\n");
    let record = t
        .fixture
        .record(&ManagedCheckoutId::parse("w11-kill-c").unwrap())
        .await
        .unwrap();
    assert!(matches!(
        record.latest_operation.action(),
        RepositoryMutationAction::ApplyBack
    ));
    assert!(matches!(
        record.latest_operation.state(),
        CheckoutOperationState::Intent
    ));
}

/// W11 (2): a full run applies added, modified, and deleted entries into the
/// original checkout, keeps the exec bit, and records `Completed`.
#[tokio::test]
async fn apply_back_roundtrip_added_modified_deleted_with_mode() {
    let t = trio("w11-roundtrip").await;
    let added = t.a.root.join("notes/new.txt");
    fs::create_dir_all(added.parent().unwrap()).unwrap();
    fs::write(&added, b"new notes\n").unwrap();
    fs::set_permissions(&added, fs::Permissions::from_mode(0o755)).unwrap();
    child_write(&t.a, "tracked.txt", b"roundtrip tracked bytes\n");
    child_remove(&t.a, "a/one.txt");
    let assignment = assigned(&["a/one.txt", "notes/new.txt", "tracked.txt"]);
    let token = CancellationToken::new();
    let (delta, receipt) = child_delta(&t.a, &assignment, &token);
    let accepted = accepted_for(&t.a, &delta, &receipt);
    let plan = plan_index(&t);
    let state = state_dir(&t);
    let applied = apply_back(
        &apply_input(&t, &plan, &state, &[accepted]),
        &token,
        ApplyBackBounds::default(),
    )
    .await
    .unwrap();
    assert_eq!(applied.applied_files(), 3);
    assert_eq!(
        fs::read_to_string(t.fixture.repo_root.join("tracked.txt")).unwrap(),
        "roundtrip tracked bytes\n"
    );
    assert!(!t.fixture.repo_root.join("a/one.txt").exists());
    let mode = fs::metadata(t.fixture.repo_root.join("notes/new.txt"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o755);
    let record = t
        .fixture
        .record(&ManagedCheckoutId::parse("w11-roundtrip-c").unwrap())
        .await
        .unwrap();
    assert!(matches!(
        record.latest_operation.action(),
        RepositoryMutationAction::ApplyBack
    ));
    assert!(matches!(
        record.latest_operation.state(),
        CheckoutOperationState::Completed
    ));
}

/// W11 (3): user drift before the run is a typed collision — nothing is
/// written, no journal directory appears, and no operation row is added.
#[tokio::test]
async fn user_edit_before_apply_is_refused_as_a_collision() {
    let t = trio("w11-collision").await;
    child_write(&t.a, "a/one.txt", b"child collision bytes\n");
    let assignment = assigned(&["a/one.txt"]);
    let token = CancellationToken::new();
    let (delta, receipt) = child_delta(&t.a, &assignment, &token);
    let accepted = accepted_for(&t.a, &delta, &receipt);
    fs::write(t.fixture.repo_root.join("a/one.txt"), b"user drift bytes\n").unwrap();
    let plan = plan_index(&t);
    let state = state_dir(&t);
    let candidate = ManagedCheckoutId::parse("w11-collision-c").unwrap();
    let before = t.fixture.record(&candidate).await.unwrap();
    let result = apply_back(
        &apply_input(&t, &plan, &state, &[accepted]),
        &token,
        ApplyBackBounds::default(),
    )
    .await;
    assert!(matches!(result, Err(ApplyBackError::Collision(_))));
    assert_eq!(
        fs::read_to_string(t.fixture.repo_root.join("a/one.txt")).unwrap(),
        "user drift bytes\n"
    );
    assert!(apply_back_latest_dir(&state, &t.fixture.job).is_none());
    let after = t.fixture.record(&candidate).await.unwrap();
    assert_eq!(before, after);
}

/// W11 (4), E16: apply-back touches only delta paths — unrelated staged
/// dirt keeps its porcelain line and its bytes.
#[tokio::test]
async fn unrelated_dirt_and_staging_are_preserved() {
    let t = trio("w11-e16").await;
    child_write(&t.a, "a/one.txt", b"child e16 bytes\n");
    let assignment = assigned(&["a/one.txt"]);
    let token = CancellationToken::new();
    let (delta, receipt) = child_delta(&t.a, &assignment, &token);
    let accepted = accepted_for(&t.a, &delta, &receipt);
    fs::write(
        t.fixture.repo_root.join("unrelated.txt"),
        b"unrelated bytes\n",
    )
    .unwrap();
    t.fixture
        .git_output(&t.fixture.repo_root, &["add", "unrelated.txt"]);
    let plan = plan_index(&t);
    let state = state_dir(&t);
    let applied = apply_back(
        &apply_input(&t, &plan, &state, &[accepted]),
        &token,
        ApplyBackBounds::default(),
    )
    .await
    .unwrap();
    assert_eq!(applied.applied_files(), 1);
    let status = t
        .fixture
        .git_output(&t.fixture.repo_root, &["status", "--porcelain"]);
    assert!(status.lines().any(|line| line == "A  unrelated.txt"));
    assert_eq!(
        fs::read(t.fixture.repo_root.join("unrelated.txt")).unwrap(),
        b"unrelated bytes\n"
    );
}

/// W11 (5): an oversize delta file is refused whole before any fs contact —
/// the original tree is untouched and the `ApplyBack` intent stays open.
#[tokio::test]
async fn oversize_leaves_the_intent_open() {
    let t = trio("w11-oversize").await;
    let big = t.a.root.join("big.bin");
    let file = fs::File::create(&big).unwrap();
    file.set_len(9 * 1024 * 1024).unwrap();
    drop(file);
    let assignment = assigned(&["big.bin"]);
    let token = CancellationToken::new();
    let (delta, receipt) = child_delta(&t.a, &assignment, &token);
    let accepted = accepted_for(&t.a, &delta, &receipt);
    let plan = plan_index(&t);
    let state = state_dir(&t);
    let result = apply_back(
        &apply_input(&t, &plan, &state, &[accepted]),
        &token,
        ApplyBackBounds::default(),
    )
    .await;
    assert!(matches!(result, Err(ApplyBackError::Apply(_))));
    assert!(!t.fixture.repo_root.join("big.bin").exists());
    let record = t
        .fixture
        .record(&ManagedCheckoutId::parse("w11-oversize-c").unwrap())
        .await
        .unwrap();
    assert!(matches!(
        record.latest_operation.action(),
        RepositoryMutationAction::ApplyBack
    ));
    assert!(matches!(
        record.latest_operation.state(),
        CheckoutOperationState::Intent
    ));
}
