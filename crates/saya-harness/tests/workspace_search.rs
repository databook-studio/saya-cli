//! Tests for the contained search surface: `glob` and `grep` over the
//! contained walk. The defining invariants: candidates are only ever paths
//! the containment walk itself resolved (never a symlink, never outside),
//! bounds fail as typed errors rather than truncating quietly, and grep
//! serves honest coverage (`files_skipped`) instead of mojibake or a prefix
//! scan that reads as full coverage.

use std::{fs, path::PathBuf};

use saya_harness::{HarnessError, workspace::Workspace};

/// Keeps a sandbox directory alive for the test and removes it afterwards.
/// The workspace root sits one level down (`outer/ws`) so outside symlink
/// targets have real places to name.
struct Sandbox {
    outer: PathBuf,
    ws: Workspace,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let outer =
            std::env::temp_dir().join(format!("saya-wssearch-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&outer);
        fs::create_dir_all(outer.join("ws")).expect("sandbox directory must create");
        let ws = Workspace::open(&outer.join("ws")).expect("workspace root must open");
        Self { outer, ws }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.outer);
    }
}

fn plant_tree(ws: &saya_harness::workspace::Workspace) {
    for path in ["a.md", "notes/b.md", "notes/deep/c.md", "code/x.rs"] {
        ws.write(path, b"content")
            .expect("contained write must succeed");
    }
}

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

#[test]
fn grep_reports_literal_hits_with_one_based_line_numbers() {
    let sandbox = Sandbox::new("grep-content");
    sandbox
        .ws
        .write("log.txt", b"one\ntwo\nneedle\nNeedle case")
        .expect("contained write must succeed");
    let outcome = sandbox
        .ws
        .grep("needle", false, 100, 100, 4096, 2_000)
        .expect("a contained grep must succeed");
    assert_eq!(outcome.files_scanned, 1);
    assert_eq!(outcome.files_skipped, 0);
    assert_eq!(outcome.matches.len(), 1, "case-sensitive by default");
    assert_eq!(outcome.matches[0].path, "log.txt");
    assert_eq!(outcome.matches[0].line, 3);
    assert_eq!(outcome.matches[0].text, "needle");
    assert!(!outcome.matches[0].truncated);
    let folded = sandbox
        .ws
        .grep("NEEDLE", true, 100, 100, 4096, 2_000)
        .expect("a contained grep must succeed");
    assert_eq!(folded.matches.len(), 2, "the flag folds both sides");
    assert_eq!(folded.matches[1].line, 4);
}

#[test]
fn grep_enforces_the_visited_and_match_bounds() {
    let sandbox = Sandbox::new("grep-bounds");
    plant_tree(&sandbox.ws);
    let visited = sandbox
        .ws
        .grep("needle", false, 3, 100, 4096, 2_000)
        .expect_err("a walk past the visited bound must refuse");
    assert!(matches!(visited, HarnessError::BoundsExceeded { .. }));
    for path in ["f1.txt", "f2.txt", "f3.txt"] {
        sandbox
            .ws
            .write(path, b"needle")
            .expect("contained write must succeed");
    }
    let matched = sandbox
        .ws
        .grep("needle", false, 100, 2, 4096, 2_000)
        .expect_err("a walk past the match bound must refuse");
    match matched {
        HarnessError::BoundsExceeded { found, max, .. } => {
            assert_eq!((found, max), (3, 2));
        }
        other => panic!("expected a typed bounds error, got: {other}"),
    }
}

/// A file bigger than the per-file read bound is skipped whole — a hit list
/// over a prefix would read as full coverage — and a non-UTF-8 file is
/// skipped rather than served as mojibake. Both land in `files_skipped`.
#[test]
fn grep_skips_oversized_and_non_utf8_files_and_counts_the_skips() {
    let sandbox = Sandbox::new("grep-skip");
    let mut oversized = vec![b'a'; 33];
    oversized.extend_from_slice(b"needle");
    sandbox
        .ws
        .write("big.txt", &oversized)
        .expect("contained write must succeed");
    sandbox
        .ws
        .write("bin.dat", b"\xff\xfe\xfd")
        .expect("contained write must succeed");
    sandbox
        .ws
        .write("small.txt", b"needle here")
        .expect("contained write must succeed");
    let outcome = sandbox
        .ws
        .grep("needle", false, 100, 100, 32, 2_000)
        .expect("a contained grep must succeed");
    assert_eq!(outcome.files_scanned, 1, "only the fully readable file");
    assert_eq!(outcome.files_skipped, 2);
    assert_eq!(outcome.matches.len(), 1);
    assert_eq!(outcome.matches[0].path, "small.txt");
}

/// One enormous line cannot blow the context: the reported text is capped at
/// exactly the line bound, on a character boundary, and the cap is reported.
#[test]
fn grep_bounds_the_reported_line_itself() {
    let sandbox = Sandbox::new("grep-line-bound");
    // Two-byte characters, so a byte-capped line can only be reported honestly
    // if the cap lands on a character boundary.
    let line = format!("needle{}", "é".repeat(1_500));
    sandbox
        .ws
        .write("wide.txt", line.as_bytes())
        .expect("contained write must succeed");
    let outcome = sandbox
        .ws
        .grep("needle", false, 100, 100, 4096, 2_000)
        .expect("a contained grep must succeed");
    assert_eq!(outcome.matches.len(), 1);
    assert!(outcome.matches[0].truncated);
    assert_eq!(outcome.matches[0].text.len(), 2_000);
    assert!(
        outcome.matches[0]
            .text
            .chars()
            .all(|character| character != char::REPLACEMENT_CHARACTER),
        "the cap must cut on a character boundary, never into mojibake"
    );
}

/// The walk's symlink discipline: an outside-pointing symlink is neither
/// visited by name nor descended into, so it can neither match nor be read.
#[cfg(unix)]
#[test]
fn the_walk_never_visits_or_descends_a_symlink() {
    use std::os::unix::fs::symlink;

    let sandbox = Sandbox::new("walk-symlink");
    fs::write(sandbox.outer.join("outside.md"), b"outside needle")
        .expect("outside plant must succeed");
    fs::create_dir_all(sandbox.outer.join("outside_dir")).expect("outside dir must create");
    fs::write(
        sandbox.outer.join("outside_dir").join("inside.md"),
        b"inside needle",
    )
    .expect("outside plant must succeed");
    symlink("../outside.md", sandbox.ws.root().join("decoy.md"))
        .expect("decoy symlink must create");
    symlink("../outside_dir", sandbox.ws.root().join("linkdir"))
        .expect("linkdir symlink must create");
    sandbox
        .ws
        .write("real/inner.md", b"real needle")
        .expect("contained write must succeed");
    let paths = sandbox
        .ws
        .glob("**/*.md", 100, 100)
        .expect("a contained glob must succeed")
        .into_iter()
        .map(|matched| matched.path)
        .collect::<Vec<_>>();
    assert_eq!(paths, vec!["real/inner.md"]);
    let outcome = sandbox
        .ws
        .grep("needle", false, 100, 100, 4096, 2_000)
        .expect("a contained grep must succeed");
    assert_eq!(outcome.files_scanned, 1);
    assert_eq!(outcome.matches.len(), 1);
    assert_eq!(outcome.matches[0].path, "real/inner.md");
}
