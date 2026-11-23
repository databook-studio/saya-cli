//! The vetted read-only set's decision table: the commands a shell call may run
//! without a question, and every shape that keeps asking. The table is the
//! specification — a widening of it is a reviewed change, never a convenience.
//!
//! Every row is read against a real workspace, because the set asks what a
//! command would read: a path has to stay inside the root and off the read
//! tool's private list, and a `git` spelling has to be read against the
//! repository's own configuration.

use std::path::Path;

use super::read_only_cases::{ASKING, QUALIFYING};
use super::read_only_fixture::Workspace;
use super::{CommandDecision, CommandRule, CommandRules, ReadOnlySet, decide};

/// Whether the default table reads the command as a vetted read-only one.
fn qualifies(root: &Path, command: &str) -> bool {
    let rules = CommandRules::default();
    decide(&rules, command, Some(root)) == CommandDecision::ReadOnly
}

/// Whether the default table leaves the command to the lane decision.
fn asks(root: &Path, command: &str) -> bool {
    let rules = CommandRules::default();
    decide(&rules, command, Some(root)) == CommandDecision::Unmatched
}

/// Every part of these commands only reads: plain programs with flags that
/// describe what to print, a path inside the workspace that the workspace
/// tools would also serve, pipelines and chains of them, and `git`'s
/// read-only subcommands over a repository that runs nothing of its own.
#[test]
fn a_vetted_read_only_command_qualifies() {
    let workspace = Workspace::new("qualifying");
    let failed: Vec<&str> = QUALIFYING
        .iter()
        .copied()
        .filter(|command| !qualifies(workspace.path(), command))
        .collect();
    assert!(
        failed.is_empty(),
        "{} of {} vetted commands would ask: {failed:?}",
        failed.len(),
        QUALIFYING.len()
    );
}

/// One shape outside the set is enough to keep the question: a write, a
/// program that is not vetted, a launcher, an environment change, an
/// interpreter's inline code, a path that leaves the workspace or names a
/// private file, an expansion, a recursive content read, or a `git` spelling
/// that runs something else.
#[test]
fn every_other_shape_keeps_asking() {
    let workspace = Workspace::new("asking");
    let failed: Vec<&str> = ASKING
        .iter()
        .copied()
        .filter(|command| !asks(workspace.path(), command))
        .collect();
    assert!(
        failed.is_empty(),
        "{} of {} unvetted commands would not ask: {failed:?}",
        failed.len(),
        ASKING.len()
    );
}

/// The set is decided by parsing the command, never by matching its text: a
/// program that merely begins with a vetted name, or is written as a path, is
/// not the vetted program.
#[test]
fn the_set_is_not_a_prefix_or_a_substring_match() {
    let workspace = Workspace::new("prefix");
    for command in [
        "lsx",
        "lsof -i",
        "catalogue",
        "gitstatus",
        "status",
        "grepview",
        "/bin/ls",
        "./ls",
        "bin/ls -la",
        "sudo ls",
        "command ls",
    ] {
        assert!(
            asks(workspace.path(), command),
            "must keep asking: {command:?}"
        );
    }
}

/// A denied program in the set is still refused: the deny list wins before the
/// set is consulted, in every mode.
#[test]
fn a_denied_program_in_the_set_is_refused() {
    let workspace = Workspace::new("deny");
    let rules = CommandRules {
        deny: vec![CommandRule::parse("ls").expect("the test rule parses")],
        ..Default::default()
    };
    let decision = decide(&rules, "ls -la", Some(workspace.path()));
    assert!(
        matches!(&decision, CommandDecision::Deny(rule) if rule.text() == "ls"),
        "a denied program is refused, not vetted: {decision:?}"
    );
    // A command the set would answer still asks when it names a denied program
    // anywhere, once the deny side cannot read its script.
    let rules = CommandRules {
        deny: vec![CommandRule::parse("rm").expect("the test rule parses")],
        ..Default::default()
    };
    assert!(matches!(
        decide(&rules, "ls && rm -rf x", Some(workspace.path())),
        CommandDecision::Deny(_)
    ));
}

/// The switch turns the set off: every command keeps the question it asked
/// before the set existed.
#[test]
fn the_switch_off_asks_for_every_vetted_command() {
    let workspace = Workspace::new("off");
    let rules = CommandRules {
        read_only_set: ReadOnlySet::Off,
        ..Default::default()
    };
    for command in ["ls -la", "git status", "cat README.md | wc -l", "pwd"] {
        assert_eq!(
            decide(&rules, command, Some(workspace.path())),
            CommandDecision::Unmatched,
            "the set is off, so `{command}` asks"
        );
    }
}

/// An operator's own allow rule still covers what the set does not, and the
/// two answers are distinct: a stated prefix is the operator's word, the set
/// is the built-in one.
#[test]
fn an_allow_rule_is_still_the_operators_own_answer() {
    let workspace = Workspace::new("allow");
    let rules = CommandRules {
        allow: vec![CommandRule::parse("npm install").expect("the test rule parses")],
        ..Default::default()
    };
    assert_eq!(
        decide(&rules, "npm install left-pad", Some(workspace.path())),
        CommandDecision::Allow
    );
}
