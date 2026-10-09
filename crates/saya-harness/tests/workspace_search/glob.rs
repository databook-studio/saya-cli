//! The glob surface over the contained walk: pattern semantics (`**` spans
//! segments and matches zero of them, `*` stays in one segment) and the
//! visited/match bounds, which refuse as typed errors rather than
//! truncating quietly.

use saya_harness::HarnessError;

use super::{Sandbox, plant_tree};

#[test]
fn glob_matches_walk_built_paths_with_glob_semantics() {
    let sandbox = Sandbox::new("glob-semantics");
    plant_tree(&sandbox.ws);
    assert_eq!(
        sandbox
            .ws
            .glob("**/*.md", 100, 100)
            .expect("a contained glob must succeed")
            .into_iter()
            .map(|matched| matched.path)
            .collect::<Vec<_>>(),
        vec!["a.md", "notes/b.md", "notes/deep/c.md"],
        "`**` spans segments and also matches zero of them"
    );
    assert_eq!(
        sandbox
            .ws
            .glob("*.md", 100, 100)
            .expect("a contained glob must succeed")
            .into_iter()
            .map(|matched| matched.path)
            .collect::<Vec<_>>(),
        vec!["a.md"],
        "`*` stays inside one segment"
    );
    assert_eq!(
        sandbox
            .ws
            .glob("notes/**", 100, 100)
            .expect("a contained glob must succeed")
            .into_iter()
            .map(|matched| matched.path)
            .collect::<Vec<_>>(),
        vec!["notes", "notes/b.md", "notes/deep", "notes/deep/c.md"],
        "a trailing `**` matches the directory and everything real under it"
    );
}

/// A walk that would visit past the visited bound refuses as a typed error —
/// a silently shortened walk would make "no matches" a lie about coverage.
#[test]
fn glob_enforces_the_visited_bound() {
    let sandbox = Sandbox::new("glob-visited");
    plant_tree(&sandbox.ws);
    let error = sandbox
        .ws
        .glob("**", 3, 100)
        .expect_err("a walk past the visited bound must refuse");
    match error {
        HarnessError::BoundsExceeded { found, max, .. } => {
            assert_eq!((found, max), (4, 3));
        }
        other => panic!("expected a typed bounds error, got: {other}"),
    }
}

#[test]
fn glob_enforces_the_match_bound() {
    let sandbox = Sandbox::new("glob-match-bound");
    plant_tree(&sandbox.ws);
    let error = sandbox
        .ws
        .glob("**/*.md", 100, 2)
        .expect_err("an over-bound match list must refuse");
    match error {
        HarnessError::BoundsExceeded { found, max, .. } => {
            assert_eq!((found, max), (3, 2), "the third match trips the bound");
        }
        other => panic!("expected a typed bounds error, got: {other}"),
    }
}
