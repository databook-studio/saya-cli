//! The bounds and the lane the workspace summary is gathered under.
//!
//! One sentence per test: a folder's shape reaches the first request only
//! inside its bounds, and a scan that has not finished never holds a message.
//! The fixture folders live under `target/`, as every composition fixture does.

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use super::{EMPTY_FOLDER, GitFacts, Limits, MAX_BYTES, MAX_ENTRIES, PARTIAL_MARK, scan};

/// A throwaway folder under `target/`, cleaned by whoever stops needing it.
fn folder(case: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("tmp-summary-{case}"));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("the fixture folder creates");
    root
}

fn write_file(folder: &std::path::Path, name: &str, body: &str) {
    fs::write(folder.join(name), body).expect("the fixture file writes");
}

/// The top level, a directory marked, the manifest and the instruction file:
/// what a first question about the folder needs, stated once.
#[test]
fn the_scan_names_the_top_level_manifest_and_instructions() {
    let folder = folder("shape");
    fs::create_dir(folder.join("src")).expect("the fixture directory creates");
    write_file(&folder, "Cargo.toml", "[package]\n");
    write_file(&folder, "AGENTS.md", "guard rails\n");
    let summary = scan(&folder, Limits::default(), None);
    assert!(
        !summary.partial,
        "a small folder is read whole: {summary:?}"
    );
    assert!(
        summary.body.contains("src/"),
        "a directory is marked with its trailing slash: {}",
        summary.body
    );
    assert!(
        summary.body.contains("Cargo.toml"),
        "the top level is named: {}",
        summary.body
    );
    assert!(
        summary.body.contains("manifest: Cargo.toml (Rust)"),
        "the manifest names the language: {}",
        summary.body
    );
    assert!(
        summary.body.contains("instructions: AGENTS.md"),
        "the instruction sources are named: {}",
        summary.body
    );
    let _ = fs::remove_dir_all(&folder);
}

/// An empty folder states that it is empty — never a listing that might be read
/// as "nothing has been read yet".
#[test]
fn an_empty_folder_says_it_is_empty() {
    let folder = folder("empty");
    let summary = scan(&folder, Limits::default(), None);
    assert!(
        !summary.partial,
        "an empty folder is read whole: {summary:?}"
    );
    assert!(
        summary.body.contains(EMPTY_FOLDER),
        "an empty folder says so: {}",
        summary.body
    );
    let _ = fs::remove_dir_all(&folder);
}

/// The entry bound: a folder past it is capped and says it is partial.
#[test]
fn the_entry_list_is_capped_and_says_so() {
    let folder = folder("capped");
    for index in 0..(MAX_ENTRIES * 3) {
        write_file(&folder, &format!("file-{index:03}.txt"), "x");
    }
    let summary = scan(&folder, Limits::default(), None);
    assert!(
        summary.partial,
        "a capped scan says it is partial: {summary:?}"
    );
    assert!(
        summary.body.contains(PARTIAL_MARK),
        "the partial flag is visible in the body: {}",
        summary.body
    );
    let entries = summary
        .body
        .split("entries (top level): ")
        .nth(1)
        .and_then(|rest| rest.split('\n').next())
        .expect("the entry line is stated");
    assert_eq!(
        entries.split(", ").count(),
        MAX_ENTRIES,
        "the listing stops at the entry bound: {entries}"
    );
    let _ = fs::remove_dir_all(&folder);
}

/// A folder of ten thousand files: the bounds hold on every axis.
#[test]
fn a_huge_folder_stays_inside_its_bounds() {
    let folder = folder("huge");
    for index in 0..10_000 {
        write_file(&folder, &format!("f{index:05}.txt"), "");
    }
    let summary = scan(&folder, Limits::default(), None);
    assert!(
        summary.partial,
        "the scan stopped at its bound: {summary:?}"
    );
    assert!(
        summary.body.len() <= MAX_BYTES,
        "the summary fits its byte bound: {} of {MAX_BYTES}",
        summary.body.len()
    );
    assert!(
        summary.body.split(", ").count() <= MAX_ENTRIES + 8,
        "the entry bound holds: {}",
        summary.body
    );
    let _ = fs::remove_dir_all(&folder);
}

/// The time bound: a scan with no time left keeps nothing it did not read and
/// says it is partial rather than claiming the folder is empty.
#[test]
fn a_scan_past_its_time_bound_says_partial() {
    let folder = folder("timeout");
    write_file(&folder, "Cargo.toml", "[package]\n");
    let limits = Limits {
        time: Duration::ZERO,
        ..Limits::default()
    };
    let summary = scan(&folder, limits, None);
    assert!(
        summary.partial,
        "a scan out of time is partial: {summary:?}"
    );
    assert!(
        !summary.body.contains(EMPTY_FOLDER),
        "a folder that was not read is not called empty: {}",
        summary.body
    );
    assert!(
        summary.body.contains(PARTIAL_MARK),
        "the partial flag is visible in the body: {}",
        summary.body
    );
    let _ = fs::remove_dir_all(&folder);
}

/// A Git folder states its branch and how many paths differ — the fact a
/// question about "what changed" would otherwise cost a call to read.
#[test]
fn a_git_folder_states_its_branch_and_changed_paths() {
    let folder = folder("git");
    fs::create_dir(folder.join(".git")).expect("the fixture git directory creates");
    write_file(&folder, ".git/HEAD", "ref: refs/heads/main\n");
    let git = GitFacts {
        branch: Some("main".to_owned()),
        changed: 3,
    };
    let summary = scan(&folder, Limits::default(), Some(git));
    assert!(
        summary.body.contains("repository on main, 3 changed paths"),
        "the branch and the changed count are stated: {}",
        summary.body
    );
    // A repository whose state could not be read is still a repository.
    let unread = scan(&folder, Limits::default(), None);
    assert!(
        unread.body.contains("git: repository"),
        "an unread repository is still named: {}",
        unread.body
    );
    let _ = fs::remove_dir_all(&folder);
}
