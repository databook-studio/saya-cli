use std::path::PathBuf;

use super::support::Q04_COMMANDS;

/// The ADR this release documents; every path and number claim below hangs
/// off it.
fn adr_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/adr-0009-native-coding-and-continuity.md")
}

fn adr_text() -> String {
    std::fs::read_to_string(adr_path())
        .expect("docs/adr-0009-native-coding-and-continuity.md exists at the repo root")
}

/// Inline code spans of the ADR — every `crates/…` and `docs/…` span must be
/// a real path in the tree, so the ADR cannot cite a file that moved.
#[test]
fn the_adr_cites_only_files_that_exist() {
    let adr = adr_text();
    let mut paths: Vec<&str> = Vec::new();
    for span in adr.split('`').skip(1).step_by(2) {
        if span.starts_with("crates/") || span.starts_with("docs/") {
            paths.push(span);
        }
    }
    assert!(
        paths.len() >= 10,
        "the ADR must cite the code it documents; found {paths:?}"
    );
    for path in paths {
        let full = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path);
        assert!(
            full.exists(),
            "the ADR cites {path}, which does not exist in the tree"
        );
    }
}

/// The ADR documents the same command set the docs sections do — pinned in
/// the What-shipped table, so an edited table row fails here even where the
/// command name appears elsewhere in the prose.
#[test]
fn the_adr_names_each_documented_command() {
    let adr = adr_text();
    let start = adr
        .find("## What shipped")
        .expect("the ADR has a What shipped section");
    let end = adr[start..]
        .find("## Bounds")
        .expect("the What shipped section is followed by Bounds")
        + start;
    let shipped = &adr[start..end];
    for line in Q04_COMMANDS {
        assert!(
            shipped.contains(line),
            "the ADR's What shipped table must name {line} among what shipped"
        );
    }
}

/// Bounds whose constants are private to `saya-cli` cannot be imported by a
/// test in another crate, so the ADR's numbers for them are pinned here
/// literally; the constants themselves are guarded behaviourally by the
/// crate's own unit tests (turn_summary_tests, ops_tests, record_block_tests,
/// diff_tests, workspace_write_tests). The needle is the ADR's
/// `value | \`CONSTANT\`` table cells, so editing either side fails here.
#[test]
fn the_adr_states_the_private_bounds_their_unit_tests_guard() {
    let adr = adr_text();
    for (value, name) in [
        ("16", "MAX_REFRESHED_FILES"),
        ("32", "REFRESH_BOUND"),
        ("65536 B", "DIFF_MAX_BYTES"),
        ("2000", "DIFF_MAX_LINES"),
        ("4000000", "DIFF_MAX_CELLS"),
        ("200", "DIFF_MAX_OUTPUT_LINES"),
        ("8", "UNFINISHED_ENTRIES"),
        ("120", "ENTRY_CHARS"),
        ("65536 B", "WORKSPACE_WRITE_MAX_BYTES"),
    ] {
        let needle = format!("{value} | `{name}`");
        assert!(
            adr.contains(&needle),
            "the ADR bounds table must state `{needle}` (private constant; \
             the code value is guarded by saya-cli's unit tests)"
        );
    }
}
