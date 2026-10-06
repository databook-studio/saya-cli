//! Unit tests for the session-surface preference adapter (M04b): the
//! session's bound root is the only worktree the forms see, no bound root
//! refuses with the shared worktree remedy, and the identity the session
//! uses is the one the headless CLI derives — one derivation, two surfaces.
//! The regressions guarded: deriving the worktree from the launch cwd in a
//! session (splitting the identity in two), and any cross-root leak. No
//! test-only seam: every test drives the same `run_in_session` both loops
//! hand their `SessionAction::Preferences` to.

use super::session::run_in_session;
use super::{PreferenceOutcome, operations, worktree_id_for};
use crate::cli::PreferencesCommand;
use crate::interactive::session_workspace::resolve_root;
use saya_store::SqliteStateStore;
use std::path::PathBuf;

fn unique_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "saya-preferences-session-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn fresh_store(label: &str) -> SqliteStateStore {
    let dir = unique_dir(label);
    std::fs::create_dir_all(&dir).unwrap();
    SqliteStateStore::new(dir.join("state.sqlite3"))
}

/// The first `pp-…` token in a listing — 35 chars, `pp-` plus 32 hex.
fn preference_id(text: &str) -> String {
    text.split_whitespace()
        .find(|token| token.len() == 35 && token.starts_with("pp-"))
        .expect("a pp- id in the listing")
        .to_owned()
}

/// A session with no bound root refuses with the shared worktree remedy —
/// never the launch cwd, never a silent empty list. Every form refuses the
/// same way, exit 2.
#[tokio::test]
async fn no_bound_root_refuses_with_the_worktree_remedy() {
    let store = fresh_store("no-root");
    for command in [
        PreferencesCommand::List,
        PreferencesCommand::Add {
            text: "prefer tabs".into(),
        },
        PreferencesCommand::ForgetAll,
    ] {
        let (code, message) = run_in_session(command, None, &store).await;
        assert_eq!(code, 2, "the refusal exits 2: {message}");
        assert!(
            message.contains("no project worktree here"),
            "the shared worktree remedy: {message}"
        );
    }
}

/// add → list → edit → forget through the session seam over a temp store
/// and a temp-dir root: the same outcomes the headless subcommand renders.
#[tokio::test]
async fn add_list_edit_forget_round_trip_through_the_session_seam() {
    let root = unique_dir("round-trip");
    std::fs::create_dir_all(&root).unwrap();
    let store = fresh_store("round-trip");

    let (code, added) = run_in_session(
        PreferencesCommand::Add {
            text: "prefer tabs over spaces".into(),
        },
        Some(&root),
        &store,
    )
    .await;
    assert_eq!(code, 0, "{added}");
    assert!(added.contains("Added preference pp-"), "{added}");

    let (code, listing) = run_in_session(PreferencesCommand::List, Some(&root), &store).await;
    assert_eq!(code, 0, "{listing}");
    assert!(listing.contains("prefer tabs over spaces"), "{listing}");
    let id = preference_id(&listing);

    let (code, edited) = run_in_session(
        PreferencesCommand::Edit {
            id: id.clone(),
            text: "prefer spaces".into(),
        },
        Some(&root),
        &store,
    )
    .await;
    assert_eq!(code, 0, "{edited}");
    assert!(edited.contains("Edited preference"), "{edited}");

    let (code, forgotten) =
        run_in_session(PreferencesCommand::Forget { id }, Some(&root), &store).await;
    assert_eq!(code, 0, "{forgotten}");
    assert!(forgotten.contains("Forgot preference"), "{forgotten}");

    let (code, empty) = run_in_session(PreferencesCommand::List, Some(&root), &store).await;
    assert_eq!(code, 0);
    assert!(empty.contains("No project preferences for"), "{empty}");
}

/// Two roots are two identities: what one adds, the other never lists.
#[tokio::test]
async fn another_root_sees_nothing_of_this_root() {
    let root_a = unique_dir("iso-a");
    let root_b = unique_dir("iso-b");
    std::fs::create_dir_all(&root_a).unwrap();
    std::fs::create_dir_all(&root_b).unwrap();
    let store = fresh_store("isolation");

    let (code, added) = run_in_session(
        PreferencesCommand::Add {
            text: "root a only".into(),
        },
        Some(&root_a),
        &store,
    )
    .await;
    assert_eq!(code, 0, "{added}");

    let (code, listing_b) = run_in_session(PreferencesCommand::List, Some(&root_b), &store).await;
    assert_eq!(code, 0, "{listing_b}");
    assert!(
        listing_b.contains("No project preferences for"),
        "{listing_b}"
    );
    assert!(!listing_b.contains("root a only"), "{listing_b}");

    let (code, listing_a) = run_in_session(PreferencesCommand::List, Some(&root_a), &store).await;
    assert_eq!(code, 0, "{listing_a}");
    assert!(listing_a.contains("root a only"), "{listing_a}");
}

/// One identity derivation feeds both surfaces: a preference added through
/// the session seam is exactly what the CLI's operations list when the CLI
/// derives its worktree the headless way (`resolve_root` walk-up plus
/// canonicalisation). Deriving the session's worktree from the launch cwd
/// instead of the bound root would split the identity in two — the
/// regression this pins.
#[tokio::test]
async fn the_session_and_the_cli_share_one_worktree_identity() {
    let base = unique_dir("identity");
    let wt = base.join("wt");
    std::fs::create_dir_all(wt.join(".git")).unwrap();
    std::fs::create_dir_all(wt.join("src")).unwrap();
    let store = fresh_store("identity");

    // The session binds the canonical root — the shape `SessionRuntime::root()`
    // returns; the CLI derives from a subdirectory cwd the headless way.
    let bound = std::fs::canonicalize(&wt).unwrap();
    let (code, added) = run_in_session(
        PreferencesCommand::Add {
            text: "prefer tabs".into(),
        },
        Some(&bound),
        &store,
    )
    .await;
    assert_eq!(code, 0, "{added}");

    let cli_derived = resolve_root(None, &wt.join("src"))
        .unwrap()
        .expect("the fake worktree resolves");
    let cli_id = worktree_id_for(&cli_derived).expect("the canonical root derives an id");
    let items = match operations::list_preferences(&store, &cli_id).await {
        PreferenceOutcome::Listed { items, .. } => items,
        other => panic!("expected Listed, got {other:?}"),
    };
    assert_eq!(items.len(), 1, "the CLI derivation sees the session's add");
    assert_eq!(items[0].text, "prefer tabs");
}
