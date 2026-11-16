use super::support::{added, fresh_store, listed, worktree};
use super::*;

/// Isolation: a preference added for worktree A is invisible, uneditable,
/// and unforgettable from worktree B (NotFound / existed=false), and B's
/// forget-all leaves A's row alone.
#[tokio::test]
async fn another_worktree_cannot_see_edit_or_forget() {
    let store = fresh_store("isolation");
    let wa = worktree("iso-a");
    let wb = worktree("iso-b");

    let record = added(&add_preference(&store, &wa, "worktree a only").await);

    let (_, items_b) = listed(&list_preferences(&store, &wb).await);
    assert!(items_b.is_empty(), "B sees nothing of A");

    match edit_preference(&store, &wb, record.id.as_str(), "hijack").await {
        PreferenceOutcome::Refused(refusal) => {
            assert_eq!(refusal, super::PreferenceRefusal::NotFound);
        }
        other => panic!("expected Refused, got {other:?}"),
    }
    match forget_preference(&store, &wb, record.id.as_str()).await {
        PreferenceOutcome::Forgotten { existed, id } => {
            assert!(!existed);
            assert_eq!(id, record.id.as_str());
        }
        other => panic!("expected Forgotten, got {other:?}"),
    }
    match forget_all_preferences(&store, &wb).await {
        PreferenceOutcome::ForgottenAll(removed) => assert_eq!(removed, 0),
        other => panic!("expected ForgottenAll, got {other:?}"),
    }
    let (_, items_a) = listed(&list_preferences(&store, &wa).await);
    assert_eq!(items_a.len(), 1, "A's preference survived B's attempts");
}

/// forget-all removes exactly its own worktree's rows.
#[tokio::test]
async fn forget_all_touches_only_its_own_worktree() {
    let store = fresh_store("forget-all");
    let wa = worktree("fa-a");
    let wb = worktree("fa-b");

    added(&add_preference(&store, &wa, "a one").await);
    added(&add_preference(&store, &wb, "b one").await);
    added(&add_preference(&store, &wb, "b two").await);

    match forget_all_preferences(&store, &wa).await {
        PreferenceOutcome::ForgottenAll(removed) => assert_eq!(removed, 1),
        other => panic!("expected ForgottenAll, got {other:?}"),
    }
    let (_, items_a) = listed(&list_preferences(&store, &wa).await);
    assert!(items_a.is_empty());
    let (_, items_b) = listed(&list_preferences(&store, &wb).await);
    assert_eq!(items_b.len(), 2, "B's preferences were untouched");
}
