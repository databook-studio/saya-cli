//! The grep surface over the contained walk: literal hits with one-based
//! line numbers, the visited/match bounds, honest coverage for files it
//! could not serve, and the reported line itself held to its bound.

use saya_harness::HarnessError;

use super::{Sandbox, plant_tree};

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
