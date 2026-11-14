//! `saya job`, `saya start`, and `saya follow-up`: the CLI spellings.
//!
//! Each one parses to the command the operation runs, and the two paths every
//! coding launch carries are stated the same way on both commands.

use clap::{CommandFactory, Parser};
use saya_cli::{Cli, CodingCommand, Command};
use std::path::PathBuf;

/// The words each lifecycle verb takes. Every one of the twenty must parse
/// identically under both spellings.
const LIFECYCLE: &[&[&str]] = &[
    &["export", "job-a"],
    &["events", "job-a"],
    &["review", "job-a", "checkout-a", "--approve"],
    &["review-existing", "job-a", "checkout-a", "--approve"],
    &[
        "apply",
        "job-a",
        "checkout-a",
        "--source-sha256",
        "digest-a",
        "--approve",
    ],
    &["remove-checkout", "job-a", "checkout-a", "--approve"],
    &["reconcile-checkout", "job-a", "checkout-a", "--approve"],
    &["retain-checkout", "job-a", "checkout-a", "--approve"],
    &["resume", "job-a", "--approve"],
    &["notes", "job-a"],
    &["accept", "job-a", "3"],
    &["promote", "job-a", "3", "keep the null check", "--approve"],
    &["list"],
    &["show", "job-a"],
    &["cancel", "job-a"],
    &["doctor"],
    &["setup"],
    &["reconcile", "job-a"],
    &["worktrees", "job-a"],
    &["rewind", "job-a", "--to", "2"],
];

/// The eight agent-shaped controls: `saya job` refuses every one of them, and
/// none may appear in its help.
const SESSION_CONTROLS: &[&str] = &[
    "approve",
    "deny",
    "follow-up",
    "message",
    "pause",
    "continue",
    "cancel-child",
    "start",
];

fn words(prefix: &[&str], tail: &[&str]) -> Vec<String> {
    prefix
        .iter()
        .chain(tail)
        .map(|word| (*word).to_string())
        .collect()
}

fn parsed(prefix: &[&str], tail: &[&str]) -> Command {
    let argv = words(prefix, tail);
    Cli::try_parse_from(&argv)
        .unwrap_or_else(|error| panic!("{argv:?} parses: {error}"))
        .command
        .unwrap_or_else(|| panic!("{argv:?} states a subcommand"))
}

fn job(tail: &[&str]) -> CodingCommand {
    match parsed(&["saya", "job"], tail) {
        Command::Job { command, .. } => command,
        other => panic!("saya job {tail:?} parses a job command, got {other:?}"),
    }
}

#[test]
fn saya_job_parses_every_lifecycle_verb() {
    for tail in LIFECYCLE {
        job(tail);
    }
}

#[test]
fn saya_start_builds_the_start_request() {
    let tail = [
        "job-a",
        "change one file",
        "--write",
        "tracked.txt",
        "--database-profile",
        "analytics",
        "--database-profile",
        "warehouse",
        "--plan",
        "--approve",
    ];
    let start = match parsed(&["saya", "start"], &tail) {
        Command::Start(start) => start,
        other => panic!("saya start parses, got {other:?}"),
    };
    assert_eq!(
        start.into_coding(),
        CodingCommand::Start {
            id: "job-a".into(),
            task: "change one file".into(),
            write_paths: vec!["tracked.txt".into()],
            database_profiles: vec!["analytics".into(), "warehouse".into()],
            tools: Vec::new(),
            plan: true,
            approve: true,
            in_place: false,
            managed: false,
        }
    );
}

#[test]
fn saya_follow_up_builds_the_follow_up_request() {
    let tail = [
        "job-a",
        "agent-a",
        "--expected-completion-sequence",
        "4",
        "check the null case",
        "--approve",
    ];
    let follow_up = match parsed(&["saya", "follow-up"], &tail) {
        Command::FollowUp(follow_up) => follow_up,
        other => panic!("saya follow-up parses, got {other:?}"),
    };
    assert_eq!(
        follow_up.into_coding(),
        CodingCommand::FollowUp {
            id: "job-a".into(),
            agent: "agent-a".into(),
            expected_completion_sequence: 4,
            instruction: "check the null case".into(),
            approve: true,
            in_place: false,
            managed: false,
        }
    );
}

/// Scripted starts name the repository and the trusted program directory, and
/// they name them the same way on every command: `saya job` and `saya start`
/// carry the same flags with the same defaults.
#[test]
fn the_two_paths_are_stated_the_same_way_on_both_lanes() {
    let defaults = (PathBuf::from("."), PathBuf::from("/usr/bin"));
    let stated = (PathBuf::from("/tmp/repo"), PathBuf::from("/opt/bin"));
    for tail in [&["list"][..], &["doctor"][..]] {
        assert_eq!(paths(parsed(&["saya", "job"], tail)), defaults);
    }
    assert_eq!(
        paths(parsed(
            &["saya", "start", "job-a", "a task", "--approve"],
            &[]
        )),
        defaults
    );
    let job = [
        "saya",
        "job",
        "--repository",
        "/tmp/repo",
        "--program-dir",
        "/opt/bin",
        "list",
    ];
    let start = [
        "saya",
        "start",
        "--repository",
        "/tmp/repo",
        "--program-dir",
        "/opt/bin",
        "job-a",
        "a task",
        "--approve",
    ];
    for argv in [&job[..], &start[..]] {
        assert_eq!(paths(parsed(argv, &[])), stated, "{argv:?}");
    }
}

/// The repository and the trusted program directory a parsed command carries.
fn paths(command: Command) -> (PathBuf, PathBuf) {
    match command {
        Command::Job {
            repository,
            program_dir,
            ..
        } => (repository, program_dir),
        Command::Start(start) => (start.repository, start.program_dir),
        other => panic!("{other:?} carries the two paths"),
    }
}

/// `saya job` is the lifecycle command: its help lists the twenty durable-job
/// verbs and none of the eight agent-shaped controls, which keep their own
/// spellings.
#[test]
fn saya_job_advertises_its_lifecycle_verbs_and_no_session_control() {
    let mut cli = Cli::command();
    let help = cli
        .find_subcommand_mut("job")
        .expect("`saya job` exists")
        .render_help()
        .to_string();
    let advertised = advertised_commands(&help);
    let expected: Vec<&str> = LIFECYCLE.iter().map(|tail| tail[0]).collect();
    for verb in &expected {
        assert!(
            advertised.contains(&verb.to_string()),
            "`saya job --help` lists {verb}: {help}"
        );
    }
    assert_eq!(
        advertised.len(),
        expected.len(),
        "`saya job --help` lists exactly its twenty verbs: {advertised:?}"
    );
    for control in SESSION_CONTROLS {
        assert!(
            !advertised.contains(&control.to_string()),
            "`saya job --help` must not advertise {control}: {advertised:?}"
        );
    }
}

/// The verbs clap renders in the `Commands:` block, one name per command:
/// a line indented two spaces whose next character is not a space. Wrapped
/// description lines are indented further, so they are not names.
fn advertised_commands(help: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_commands = false;
    for line in help.lines() {
        if line.trim_end() == "Commands:" {
            in_commands = true;
            continue;
        }
        if !in_commands {
            continue;
        }
        if !line.is_empty() && !line.starts_with(' ') {
            // A new section header (`Options:`) ends the block.
            break;
        }
        let rest = line.strip_prefix("  ").unwrap_or_default();
        if rest.is_empty() || rest.starts_with(' ') {
            continue;
        }
        if let Some(name) = rest.split_whitespace().next() {
            // clap's own `help` subcommand is not one of the command's verbs.
            if name != "help" {
                names.push(name.to_string());
            }
        }
    }
    names
}
