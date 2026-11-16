//! The in-place reference claims are executable: every flag the pages state
//! parses on the real surface, and the pages state the guarantees the code
//! holds — the session binding, the concurrent-edit journal, and the bounds.

use clap::Parser;
use saya_cli::Cli;

const COMMANDS: &str = include_str!("../../../../docs/commands.md");
const CAPABILITIES: &str = include_str!("../../../../docs/coding-capabilities.md");

fn parses(argv: &[&str]) -> bool {
    Cli::try_parse_from(argv).is_ok()
}

/// One list item's text, joined across its continuation lines so a flag that
/// wraps is still read as part of the item it documents.
fn bullet(page: &str, prefix: &str) -> String {
    let mut lines = page
        .lines()
        .skip_while(|line| !line.trim_start().starts_with(prefix));
    let first = lines
        .next()
        .unwrap_or_else(|| panic!("the page states `{prefix}`"));
    let mut text = first.to_owned();
    for line in lines {
        if line.trim().is_empty() || line.trim_start().starts_with("- `saya ") {
            break;
        }
        text.push(' ');
        text.push_str(line.trim());
    }
    text
}

#[test]
fn in_place_docs_claims() {
    assert!(parses(&[
        "saya",
        "start",
        "job-a",
        "task",
        "--in-place",
        "--approve"
    ]));
    assert!(parses(&[
        "saya",
        "start",
        "job-a",
        "task",
        "--managed",
        "--approve"
    ]));
    assert!(parses(&[
        "saya",
        "follow-up",
        "job-a",
        "agent-a",
        "--expected-completion-sequence",
        "1",
        "next",
        "--in-place",
        "--approve",
    ]));
    assert!(parses(&[
        "saya",
        "follow-up",
        "job-a",
        "agent-a",
        "--expected-completion-sequence",
        "1",
        "next",
        "--managed",
        "--approve",
    ]));
    assert!(parses(&[
        "saya", "job", "rewind", "job-a", "--to", "1", "--force"
    ]));
    assert!(parses(&[
        "saya",
        "job",
        "rewind",
        "job-a",
        "--baseline",
        "--force"
    ]));

    let start = bullet(COMMANDS, "- `saya start");
    assert!(
        start.contains("--in-place") && start.contains("--managed"),
        "the start line states both mode flags: {start}"
    );
    assert!(
        start.contains("in place"),
        "the start line states the in-place default: {start}"
    );
    let follow_up = bullet(COMMANDS, "- `saya follow-up");
    assert!(
        follow_up.contains("--in-place") && follow_up.contains("--managed"),
        "the follow-up line states both mode flags: {follow_up}"
    );
    assert!(
        COMMANDS.contains("--force"),
        "the job page states --force for rewind"
    );

    assert!(
        CAPABILITIES.contains("binds it as the workspace"),
        "the session binding is stated"
    );
    assert!(
        CAPABILITIES.contains("foreign_changes"),
        "review's foreign mark is stated"
    );
    assert!(
        CAPABILITIES.contains("foreign_paths"),
        "rewind's foreign report is stated"
    );
    assert!(
        CAPABILITIES.contains("--force"),
        "rewind's override is stated"
    );
    assert!(
        CAPABILITIES.contains("edits the stated folder in place by default"),
        "the in-place default is stated"
    );
    assert!(
        CAPABILITIES.contains("--managed"),
        "the managed opt-out is stated"
    );
    assert!(
        CAPABILITIES.contains("no file-count cap"),
        "the absence of a whole-tree file cap is stated"
    );
    assert!(
        CAPABILITIES.contains("4,096 paths"),
        "the before-image path bound is stated"
    );
}
