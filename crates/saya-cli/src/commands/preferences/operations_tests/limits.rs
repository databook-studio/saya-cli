use super::support::{fresh_store, listed, worktree};
use super::*;

/// Text over the 1 KiB cap, empty text, and control-character text are all
/// refused as not accepted.
#[tokio::test]
async fn oversize_empty_and_control_character_text_are_refused() {
    let store = fresh_store("text");
    let wt = worktree("text");

    for bad in [
        "x".repeat(1025),
        String::new(),
        format!("bad{}bell", char::from(7)),
    ] {
        match add_preference(&store, &wt, &bad).await {
            PreferenceOutcome::Refused(refusal) => {
                assert_eq!(refusal, super::PreferenceRefusal::TextNotAccepted);
            }
            other => panic!("expected Refused for {:?}, got {other:?}", bad.len()),
        }
    }
    let (_, items) = listed(&list_preferences(&store, &wt).await);
    assert!(items.is_empty(), "nothing was stored");
}

/// The 64-preference count cap: 64 adds succeed, the 65th is TooMany.
#[tokio::test]
async fn count_limit_the_65th_add_refuses_as_too_many() {
    let store = fresh_store("count");
    let wt = worktree("count");

    for i in 0..saya_types::MAX_PROJECT_PREFERENCES {
        let text = format!("preference number {i}");
        assert!(
            matches!(
                add_preference(&store, &wt, &text).await,
                PreferenceOutcome::Added(_)
            ),
            "add {i} of 64 must succeed"
        );
    }
    match add_preference(&store, &wt, "one too many").await {
        PreferenceOutcome::Refused(refusal) => {
            assert_eq!(refusal, super::PreferenceRefusal::TooMany);
        }
        other => panic!("expected Refused, got {other:?}"),
    }
    let (_, items) = listed(&list_preferences(&store, &wt).await);
    assert_eq!(items.len(), saya_types::MAX_PROJECT_PREFERENCES);
}

/// The 16 KiB aggregate cap: sixteen 1 KiB texts fill it exactly, the 17th
/// add is TooMuchText (the count is far below 64, so the two refusals
/// distinguish).
#[tokio::test]
async fn aggregate_limit_the_17th_kib_text_refuses_as_too_much_text() {
    let store = fresh_store("aggregate");
    let wt = worktree("aggregate");

    for i in 0..16 {
        let text = format!("pref-{i:02} {}", "x".repeat(1016));
        assert_eq!(text.len(), 1024, "exactly 1 KiB of plain text");
        assert!(
            matches!(
                add_preference(&store, &wt, &text).await,
                PreferenceOutcome::Added(_)
            ),
            "add {i} of 16 KiB texts must succeed"
        );
    }
    match add_preference(&store, &wt, "over the aggregate").await {
        PreferenceOutcome::Refused(refusal) => {
            assert_eq!(refusal, super::PreferenceRefusal::TooMuchText);
        }
        other => panic!("expected Refused, got {other:?}"),
    }
}
