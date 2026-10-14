//! W10 guard tests: the apply-back preflight refuses real user drift in the
//! original checkout and provably mutates nothing.

use saya_agent::CancellationToken;
use saya_harness::coding::integration::{CollisionKind, PlanError, PlanIndex, PreflightError};

use super::apply_support::{accepted_for, assigned, trio};
use super::support_w10::{
    child_delta, guard, original_state, plan_index, user_remove, user_stage, user_symlink,
    user_write,
};

#[tokio::test]
async fn user_edit_after_snapshot_blocks_apply() {
    let t = trio("w10-user-edit").await;
    let token = CancellationToken::new();
    let (delta, receipt) = child_delta(
        &t.a,
        "a/one.txt",
        b"child edit\n",
        &assigned(&["a"]),
        &token,
    );
    let accepted = [accepted_for(&t.a, &delta, &receipt)];
    user_write(&t, "a/one.txt", b"user edit\n");
    user_write(&t, "unrelated.txt", b"unrelated bytes\n");
    let before = original_state(&t);
    let result = guard(&t, &token, &accepted).await;
    assert!(!result.is_clean());
    assert_eq!(result.checked(), 1);
    assert_eq!(result.overflow(), 0);
    let collision = result.collisions().first().unwrap();
    assert_eq!(collision.kind(), CollisionKind::UserEdited);
    assert_eq!(collision.path(), "a/one.txt");
    assert_eq!(before, original_state(&t));
}

#[tokio::test]
async fn user_deleted_blocks_and_reports() {
    let t = trio("w10-user-deleted").await;
    let token = CancellationToken::new();
    let (delta, receipt) = child_delta(
        &t.a,
        "a/one.txt",
        b"child edit\n",
        &assigned(&["a"]),
        &token,
    );
    let accepted = [accepted_for(&t.a, &delta, &receipt)];
    user_write(&t, "unrelated.txt", b"unrelated bytes\n");
    user_stage(&t, "unrelated.txt");
    user_remove(&t, "a/one.txt");
    let before = original_state(&t);
    let result = guard(&t, &token, &accepted).await;
    assert!(!result.is_clean());
    assert_eq!(result.overflow(), 0);
    let collision = result.collisions().first().unwrap();
    assert_eq!(collision.kind(), CollisionKind::UserDeleted);
    assert_eq!(collision.path(), "a/one.txt");
    assert_eq!(before, original_state(&t));
}

#[tokio::test]
async fn user_added_untracked_collision_refused() {
    let t = trio("w10-user-added").await;
    let token = CancellationToken::new();
    let (delta, receipt) = child_delta(
        &t.a,
        "notes/n.txt",
        b"child notes\n",
        &assigned(&["notes"]),
        &token,
    );
    let accepted = [accepted_for(&t.a, &delta, &receipt)];
    user_write(&t, "notes/n.txt", b"user notes\n");
    user_write(&t, "unrelated.txt", b"unrelated bytes\n");
    let before = original_state(&t);
    let result = guard(&t, &token, &accepted).await;
    assert!(!result.is_clean());
    assert_eq!(result.overflow(), 0);
    let collision = result.collisions().first().unwrap();
    assert_eq!(collision.kind(), CollisionKind::UserAddedUntracked);
    assert_eq!(collision.path(), "notes/n.txt");
    assert_eq!(before, original_state(&t));
}

#[tokio::test]
async fn index_staged_change_refused() {
    let t = trio("w10-user-staged").await;
    let token = CancellationToken::new();
    let (delta, receipt) = child_delta(
        &t.a,
        "tracked.txt",
        b"child tracked edit\n",
        &assigned(&["tracked.txt"]),
        &token,
    );
    let accepted = [accepted_for(&t.a, &delta, &receipt)];
    user_stage(&t, "tracked.txt");
    user_write(&t, "unrelated.txt", b"unrelated bytes\n");
    let before = original_state(&t);
    let result = guard(&t, &token, &accepted).await;
    assert!(!result.is_clean());
    assert_eq!(result.overflow(), 0);
    let collision = result.collisions().first().unwrap();
    assert_eq!(collision.kind(), CollisionKind::IndexStagedDifferent);
    assert_eq!(collision.path(), "tracked.txt");
    assert_eq!(before, original_state(&t));
}

#[tokio::test]
async fn type_change_refused() {
    let t = trio("w10-user-symlink").await;
    let token = CancellationToken::new();
    let (delta, receipt) = child_delta(
        &t.a,
        "a/one.txt",
        b"child edit\n",
        &assigned(&["a"]),
        &token,
    );
    let accepted = [accepted_for(&t.a, &delta, &receipt)];
    user_symlink(&t, "a/one.txt", "elsewhere.txt");
    user_write(&t, "unrelated.txt", b"unrelated bytes\n");
    let before = original_state(&t);
    let result = guard(&t, &token, &accepted).await;
    assert!(!result.is_clean());
    assert_eq!(result.overflow(), 0);
    let collision = result.collisions().first().unwrap();
    assert_eq!(collision.kind(), CollisionKind::TypeChanged);
    assert_eq!(collision.path(), "a/one.txt");
    assert_eq!(before, original_state(&t));
}

#[tokio::test]
async fn clean_preflight_passes_and_mutates_nothing() {
    let t = trio("w10-clean").await;
    let token = CancellationToken::new();
    let (delta, receipt) = child_delta(
        &t.a,
        "a/one.txt",
        b"child edit\n",
        &assigned(&["a"]),
        &token,
    );
    let accepted = [accepted_for(&t.a, &delta, &receipt)];
    user_write(&t, "unrelated.txt", b"unrelated bytes\n");
    user_stage(&t, "unrelated.txt");
    let staged = t
        .fixture
        .git_output(&t.fixture.repo_root, &["ls-files", "-s"]);
    let before = original_state(&t);
    let result = guard(&t, &token, &accepted).await;
    assert!(result.is_clean());
    assert_eq!(result.checked(), 1);
    assert!(result.collisions().is_empty());
    assert_eq!(result.overflow(), 0);
    assert_eq!(before, original_state(&t));
    assert_eq!(
        staged,
        t.fixture
            .git_output(&t.fixture.repo_root, &["ls-files", "-s"])
    );
}

#[tokio::test]
async fn collision_and_error_debug_stay_path_free() {
    let t = trio("w10-redaction").await;
    let token = CancellationToken::new();
    let (delta, receipt) = child_delta(
        &t.a,
        "a/one.txt",
        b"child edit\n",
        &assigned(&["a"]),
        &token,
    );
    let accepted = [accepted_for(&t.a, &delta, &receipt)];
    user_write(&t, "a/one.txt", b"user edit\n");
    let result = guard(&t, &token, &accepted).await;
    let collision = result.collisions().first().unwrap();
    let collision_debug = format!("{collision:?}");
    let result_debug = format!("{result:?}");
    let error = PreflightError::from(PlanError::DigestMismatch);
    let error_debug = format!("{error:?} {error}");
    assert!(!collision_debug.contains("one.txt"));
    assert!(!result_debug.contains("one.txt"));
    assert!(!error_debug.contains("one.txt"));
}

#[tokio::test]
async fn plan_digest_mismatch_refuses() {
    let t = trio("w10-plan-digest").await;
    let plan = plan_index(&t);
    assert!(plan.index_entry("tracked.txt").is_some());
    assert!(plan.status_fact("tracked.txt").is_some());
    let sha = t.fixture.plan.manifest_sha256();
    let bytes = t.fixture.plan.canonical_bytes();
    let mut tampered = bytes.to_vec();
    let last = tampered.len() - 1;
    tampered[last] ^= 0xFF;
    assert!(matches!(
        PlanIndex::parse(sha, &tampered),
        Err(PlanError::DigestMismatch)
    ));
}
