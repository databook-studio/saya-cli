//! Unit tests for the shared project-preference operations (M04a).
//!
//! The contract under test: preferences are explicit, bounded, per-worktree,
//! secret-free, never created by the model, and managed without a database
//! profile. The regressions guarded: accepting an unvalidated id (any
//! malformed id must be refused before the store is touched), and any
//! cross-worktree leak (another worktree cannot see, edit, or forget a
//! preference it does not hold). No test-only seam: every test drives the
//! same `pub(crate)` operations the CLI subcommand renders and the slash
//! packet will reuse.

use super::operations::{
    add_preference, edit_preference, forget_all_preferences, forget_preference, list_preferences,
};
use super::{PreferenceOutcome, render};
use crate::interactive::session_continuation::now_unix_ms;
use saya_store::SqliteStateStore;
use saya_types::{
    PROJECT_PREFERENCE_VERSION, PreferenceId, PreferenceSource, ProjectPreference, WorktreeId,
};

fn worktree(label: &str) -> WorktreeId {
    WorktreeId::from_canonical_path(&format!("/tmp/saya-pref-unit/{label}")).unwrap()
}

fn fresh_store(label: &str) -> SqliteStateStore {
    let dir = std::env::temp_dir().join(format!(
        "saya-preferences-unit-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    SqliteStateStore::new(dir.join("state.sqlite3"))
}

fn added(outcome: &PreferenceOutcome) -> ProjectPreference {
    match outcome {
        PreferenceOutcome::Added(record) => record.clone(),
        other => panic!("expected Added, got {other:?}"),
    }
}

fn listed(outcome: &PreferenceOutcome) -> (String, Vec<ProjectPreference>) {
    match outcome {
        PreferenceOutcome::Listed {
            worktree_label,
            items,
        } => (worktree_label.clone(), items.clone()),
        other => panic!("expected Listed, got {other:?}"),
    }
}

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

fn added_or_edited(outcome: &PreferenceOutcome) -> ProjectPreference {
    match outcome {
        PreferenceOutcome::Added(record) | PreferenceOutcome::Edited(record) => record.clone(),
        other => panic!("expected Added or Edited, got {other:?}"),
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

/// Rendering: a multi-line, tab-carrying text lists on ONE line (newlines
/// become ` ⏎ `, tabs a space) so the listing stays line-per-preference.
#[test]
fn list_renders_a_multiline_text_on_one_line() {
    let now = now_unix_ms();
    let record = ProjectPreference {
        version: PROJECT_PREFERENCE_VERSION,
        id: PreferenceId::parse("pp-00000000000000000000000000000000").unwrap(),
        worktree: worktree("render"),
        text: "line one\nline two\ttail".to_owned(),
        source: PreferenceSource::UserExplicit,
        created_unix_ms: now - 5_000,
        updated_unix_ms: now,
    };
    let outcome = PreferenceOutcome::Listed {
        worktree_label: worktree("render").as_str().to_owned(),
        items: vec![record],
    };
    let (code, message) = render::render(&outcome);
    assert_eq!(code, 0);
    assert!(
        message.contains("line one ⏎ line two tail"),
        "one line, newline as ⏎, tab as space: {message}"
    );
    assert!(
        message.contains("Project preferences for "),
        "the header names the worktree: {message}"
    );
    assert!(message.contains("(1 of 64)"), "the count header: {message}");
    assert!(
        message.contains(saya_types::MAX_PROJECT_PREFERENCES.to_string().as_str()),
        "the cap is stated: {message}"
    );
    assert!(
        message.contains("They never grant permissions"),
        "the retention sentence rides list: {message}"
    );
}

/// Rendering: an empty listing exits 0 with the add hint (quoted form), and
/// every refusal exits 2.
#[test]
fn empty_list_hints_and_every_refusal_exits_two() {
    let outcome = PreferenceOutcome::Listed {
        worktree_label: "wt-empty".to_owned(),
        items: vec![],
    };
    let (code, message) = render::render(&outcome);
    assert_eq!(code, 0);
    assert!(
        message.contains("No project preferences for wt-empty."),
        "{message}"
    );
    assert!(
        message.contains(concat!("saya preferences add ", "\"", "<text>", "\"")),
        "the add hint quotes the placeholder: {message}"
    );

    for refusal in [
        super::PreferenceRefusal::NoWorktree,
        super::PreferenceRefusal::TextNotAccepted,
        super::PreferenceRefusal::TooMany,
        super::PreferenceRefusal::TooMuchText,
        super::PreferenceRefusal::NotFound,
        super::PreferenceRefusal::InvalidId,
        super::PreferenceRefusal::Unavailable,
    ] {
        let (code, message) = render::render(&PreferenceOutcome::Refused(refusal));
        assert_eq!(code, 2, "{refusal:?} must exit 2, message: {message}");
        assert!(!message.trim().is_empty(), "{refusal:?} states a remedy");
    }

    let (code, message) = render::render(&PreferenceOutcome::Refused(
        super::PreferenceRefusal::NoWorktree,
    ));
    assert_eq!(code, 2);
    assert!(
        message.contains("no project worktree here"),
        "the no-worktree remedy: {message}"
    );
}
