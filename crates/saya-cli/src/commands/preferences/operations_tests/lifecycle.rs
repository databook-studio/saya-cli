use super::support::{added, added_or_edited, fresh_store, listed, worktree};
use super::*;

/// add → list → edit → forget round trip: the id is stable across the edit,
/// the text replaces in place, and the forget reports what existed.
#[tokio::test]
async fn add_list_edit_forget_round_trip() {
    let store = fresh_store("round-trip");
    let wt = worktree("round-trip");

    let record = added(&add_preference(&store, &wt, "prefer tabs").await);
    assert_eq!(record.text, "prefer tabs");
    assert_eq!(record.worktree.as_str(), wt.as_str());
    assert_eq!(record.source, PreferenceSource::UserExplicit);
    assert!(record.created_unix_ms > 0);
    assert_eq!(record.updated_unix_ms, record.created_unix_ms);

    let (label, items) = listed(&list_preferences(&store, &wt).await);
    assert_eq!(label, wt.as_str());
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id.as_str(), record.id.as_str());

    let after_edit =
        added_or_edited(&edit_preference(&store, &wt, record.id.as_str(), "prefer spaces").await);
    assert_eq!(after_edit.id.as_str(), record.id.as_str());
    assert_eq!(after_edit.text, "prefer spaces");
    assert_eq!(after_edit.created_unix_ms, record.created_unix_ms);
    assert!(after_edit.updated_unix_ms >= record.created_unix_ms);

    match forget_preference(&store, &wt, record.id.as_str()).await {
        PreferenceOutcome::Forgotten { existed, id } => {
            assert!(existed);
            assert_eq!(id, record.id.as_str());
        }
        other => panic!("expected Forgotten, got {other:?}"),
    }
    let (_, items) = listed(&list_preferences(&store, &wt).await);
    assert!(items.is_empty(), "the forgotten preference is gone");

    match forget_preference(&store, &wt, record.id.as_str()).await {
        PreferenceOutcome::Forgotten { existed, .. } => assert!(!existed),
        other => panic!("expected Forgotten, got {other:?}"),
    }
}

/// Secret-shaped text is refused before anything is stored: the redactor
/// would rewrite it, so the store must never hold it, redacted or not.
#[tokio::test]
async fn secret_shaped_text_is_refused_and_nothing_is_stored() {
    let store = fresh_store("secret");
    let wt = worktree("secret");

    match add_preference(&store, &wt, "api_key=abc123").await {
        PreferenceOutcome::Refused(refusal) => {
            assert_eq!(refusal, super::PreferenceRefusal::TextNotAccepted);
        }
        other => panic!("expected Refused, got {other:?}"),
    }
    let (_, items) = listed(&list_preferences(&store, &wt).await);
    assert!(items.is_empty(), "nothing was stored");
}

/// Malformed ids (`nope`, `pp-xyz`, `../x`) are refused as InvalidId before
/// any store call: the listing afterwards still shows the store untouched.
#[tokio::test]
async fn invalid_id_shapes_refuse_without_touching_the_store() {
    let store = fresh_store("ids");
    let wt = worktree("ids");

    for id in ["nope", "pp-xyz", "../x"] {
        match edit_preference(&store, &wt, id, "new text").await {
            PreferenceOutcome::Refused(refusal) => {
                assert_eq!(refusal, super::PreferenceRefusal::InvalidId);
            }
            other => panic!("expected Refused for edit of {id:?}, got {other:?}"),
        }
        match forget_preference(&store, &wt, id).await {
            PreferenceOutcome::Refused(refusal) => {
                assert_eq!(refusal, super::PreferenceRefusal::InvalidId);
            }
            other => panic!("expected Refused for forget of {id:?}, got {other:?}"),
        }
    }
    let (_, items) = listed(&list_preferences(&store, &wt).await);
    assert!(items.is_empty(), "the store was never touched");
}
