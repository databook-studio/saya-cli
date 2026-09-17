//! E5 pins: the `workspace_edit` failure contract lives in the tool's own
//! doc comment, the D15 amendment lands verbatim, and no document still
//! claims an edit tool is absent or rejected.

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|parent| parent.parent())
        .expect("saya-cli lives two levels below the repo root")
        .to_path_buf()
}

fn read_repo(relative: &str) -> String {
    let path = repo_root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("must read {relative}: {error}"))
}

/// The `workspace_edit` module doc comment, read as source text: the
/// failure contract's home, pinned row by row.
fn workspace_edit_module_source() -> String {
    read_repo("crates/saya-cli/src/agent/tools/database_tools/workspace_edit.rs")
}

/// Every row of the DESIGN §4 failure contract must appear in the tool's
/// own module doc comment, so a future behaviour edit without the doc
/// fails this test.
#[test]
fn the_tool_doc_comment_carries_every_failure_contract_row() {
    let source = workspace_edit_module_source();
    // The doc comment is the `//!` block at the top of the module: every
    // contract row must live there, not in a behaviour comment lower in
    // the file.
    let doc: String = source
        .lines()
        .take_while(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with("//!") || trimmed.is_empty()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !doc.trim().is_empty(),
        "the workspace_edit module must open with a doc comment"
    );
    // The doc comment wraps across lines, so match against whitespace-folded
    // text: a contract row split over a line break still counts.
    let folded = doc
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    for row in [
        // Zero matches.
        "zero matches",
        // Multiple matches.
        "multiple matches",
        // Moved anchor.
        "moved anchor",
        // Over-bound replacement/chunk.
        "over the bound",
        // Offset mismatch (append).
        "offset mismatch",
        // Mid-edit truncation / truncated arguments.
        "truncated",
        // Non-UTF-8 target.
        "non-UTF-8",
        // Concurrent writers.
        "concurrent writers",
        // Empty anchor.
        "empty anchor",
    ] {
        assert!(
            folded.contains(&row.to_lowercase()),
            "the tool doc comment must carry the failure-contract row {row:?}"
        );
    }
    assert!(
        folded.contains("writes nothing"),
        "the tool doc comment must state the contract's consequence — refusals write nothing"
    );
}

/// The D15 amendment (§9 wording) lands verbatim in the decision record,
/// replacing the falsified half while keeping the failure-mode analysis.
#[test]
fn d15_is_amended_verbatim_and_the_old_ruling_is_gone() {
    let design = read_repo("HARNESS-DESIGN.md");
    for fragment in [
        "`workspace_edit` admitted (anchored `replace` with exactly-one-match",
        "refusal + offset-checked `append`, one tool sharing one atomic contained",
        "operation): `workspace_write` alone cannot express chunked writes or modify",
        "files over 64 KiB. The exact-match failure modes D15 named are answered by",
        "the failure contract (zero/multi/moved/truncation all refuse loudly, never",
        "partial) — see DESIGN.md §4. Whole-file `workspace_write` stays for small",
        "artefacts.",
    ] {
        assert!(
            design.contains(fragment),
            "HARNESS-DESIGN.md must carry the §9 amendment verbatim; missing {fragment:?}"
        );
    }
    assert!(
        !design.contains("exact-match editing adds failure modes, not capability"),
        "the falsified D15 half (\"not capability\") must be retired from HARNESS-DESIGN.md"
    );
}

/// No document still claims an edit tool is absent or rejected: asserted
/// over every file that made the claim, not one file.
#[test]
fn no_document_still_claims_an_edit_tool_is_absent_or_rejected() {
    for relative in [
        "HARNESS-DESIGN.md",
        "crates/saya-cli/src/agent/tools/database_tools/definitions.rs",
    ] {
        let text = read_repo(relative);
        assert!(
            !text.contains("edit_file` rejected for v1"),
            "{relative} must not reject an edit tool for v1"
        );
        assert!(
            !text.contains("edit_file` is deliberately absent"),
            "{relative} must not claim an edit tool is absent"
        );
        assert!(
            !text.contains("keeps `edit_file`"),
            "{relative} must not keep an edit tool absent"
        );
        assert!(
            !text.contains("whole-file writes only"),
            "{relative} must not claim whole-file writes are the only write"
        );
    }
}

/// The read-only approval policy denies the write-shaped effect the tool
/// declares: the approval-matrix row for the write-shaped tool.
#[test]
fn the_approval_matrix_denies_the_write_shaped_effect() {
    use saya_agent::{LocalStateEffect, ToolEffect};
    let effect = ToolEffect {
        database_data: false,
        external_side_effect: false,
        requires_approval: false,
        local_state: LocalStateEffect::WriteWorkspace,
    };
    assert!(
        !saya_agent::read_only_permits(&effect),
        "the approval matrix must deny the write-shaped WriteWorkspace effect"
    );
}
