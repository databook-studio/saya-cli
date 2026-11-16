//! The glob surface over the contained walk: pattern semantics (`**` spans
//! segments and matches zero of them, `*` stays in one segment), the visited
//! bound, which refuses as a typed error rather than truncating quietly, and
//! the match bound, which serves the first matches and counts the rest.

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
            .matches
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
            .matches
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
            .matches
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

/// A match list past the match bound is served short with the rest counted:
/// the caller gets `max_matches` paths and how many it did not see, instead
/// of a failed call it must answer by narrowing at random. Which paths
/// survive is the walk's own order — the point is that the count is exact and
/// the answer is served, never that a particular subset wins.
#[test]
fn glob_truncates_past_the_match_bound_and_counts_the_rest() {
    let sandbox = Sandbox::new("glob-match-bound");
    plant_tree(&sandbox.ws);
    let outcome = sandbox
        .ws
        .glob("**/*.md", 100, 2)
        .expect("an over-bound match list must truncate, not refuse");
    let served: Vec<&str> = outcome
        .matches
        .iter()
        .map(|matched| matched.path.as_str())
        .collect();
    assert_eq!(served.len(), 2, "the bound caps what is served");
    assert!(
        served
            .iter()
            .all(|path| ["a.md", "notes/b.md", "notes/deep/c.md"].contains(path)),
        "only real matches are served: {served:?}"
    );
    let mut sorted = served.clone();
    sorted.sort_unstable();
    assert_eq!(served, sorted, "the served matches are sorted by path");
    assert!(outcome.truncated);
    assert_eq!(outcome.omitted, 1, "the third match is counted, not lost");
}
