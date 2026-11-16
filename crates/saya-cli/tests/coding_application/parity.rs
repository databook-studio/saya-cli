//! The session spellings are the CLI spellings: `/job <tail>` parses to the
//! command `saya job <tail>` parses, `/start` to `saya start`, and
//! `/follow-up` to `saya follow-up`. One grammar, no second copy.

use clap::Parser;
use saya_cli::{Cli, CodingCommand, Command, SlashCommand, parse_slash_command};

fn slash_command(line: &str) -> CodingCommand {
    match parse_slash_command(line).unwrap().unwrap() {
        SlashCommand::Coding(command) => command,
        _ => panic!("{line} parses a job command"),
    }
}

/// `saya job <words>` parsed into the one command the operation runs.
fn job_command(words: &[&str]) -> CodingCommand {
    let argv: Vec<String> = ["saya", "job"]
        .into_iter()
        .chain(words.iter().copied())
        .map(str::to_string)
        .collect();
    match Cli::try_parse_from(&argv).unwrap().command {
        Some(Command::Job { command, .. }) => command,
        other => panic!("{argv:?} parses a job command, got {other:?}"),
    }
}

/// `saya start` parsed into the one command the operation runs.
fn start_command(words: &[&str]) -> CodingCommand {
    let argv: Vec<String> = ["saya", "start"]
        .into_iter()
        .chain(words.iter().copied())
        .map(str::to_string)
        .collect();
    match Cli::try_parse_from(&argv).unwrap().command {
        Some(Command::Start(start)) => start.into_coding(),
        other => panic!("{argv:?} parses a start command, got {other:?}"),
    }
}

/// `saya follow-up` parsed into the one command the operation runs.
fn follow_up_command(words: &[&str]) -> CodingCommand {
    let argv: Vec<String> = ["saya", "follow-up"]
        .into_iter()
        .chain(words.iter().copied())
        .map(str::to_string)
        .collect();
    match Cli::try_parse_from(&argv).unwrap().command {
        Some(Command::FollowUp(follow_up)) => follow_up.into_coding(),
        other => panic!("{argv:?} parses a follow-up command, got {other:?}"),
    }
}

#[test]
fn job_lifecycle_verbs_parse_identically_on_cli_and_slash() {
    for tail in [
        "show job-a",
        "cancel job-a",
        "review job-a checkout-a --approve",
        "retain-checkout job-a checkout-a --approve",
        "rewind job-a --to 2",
    ] {
        let words: Vec<&str> = tail.split_whitespace().collect();
        assert_eq!(
            slash_command(&format!("/job {tail}")),
            job_command(&words),
            "/job {tail}"
        );
    }
}

#[test]
fn start_slash_mirrors_saya_start() {
    let words = [
        "job-a",
        "change one file",
        "--write",
        "tracked.txt",
        "--database-profile",
        "analytics",
        "--database-profile",
        "warehouse",
        "--approve",
    ];
    assert_eq!(
        slash_command(
            "/start job-a 'change one file' --write tracked.txt \
             --database-profile analytics --database-profile warehouse --approve"
        ),
        start_command(&words)
    );
}

#[test]
fn follow_up_slash_mirrors_saya_follow_up() {
    let words = [
        "job-a",
        "agent-a",
        "--expected-completion-sequence",
        "4",
        "check the null case",
        "--approve",
    ];
    assert_eq!(
        slash_command(
            "/follow-up job-a agent-a --expected-completion-sequence 4 \
             'check the null case' --approve"
        ),
        follow_up_command(&words)
    );
}

/// The six session-only controls have no CLI spelling; each parses to its one
/// command.
#[test]
fn session_controls_parse_their_own_spellings() {
    assert_eq!(
        slash_command("/approve job-a"),
        CodingCommand::Approve { id: "job-a".into() }
    );
    assert_eq!(
        slash_command("/deny job-a"),
        CodingCommand::Deny { id: "job-a".into() }
    );
    assert_eq!(
        slash_command("/message job-a agent-a 'check the null case'"),
        CodingCommand::Message {
            id: "job-a".into(),
            agent: "agent-a".into(),
            text: "check the null case".into(),
        }
    );
    assert_eq!(
        slash_command("/pause job-a agent-a"),
        CodingCommand::Pause {
            id: "job-a".into(),
            agent: "agent-a".into(),
        }
    );
    assert_eq!(
        slash_command("/continue job-a agent-a"),
        CodingCommand::Continue {
            id: "job-a".into(),
            agent: "agent-a".into(),
        }
    );
    assert_eq!(
        slash_command("/cancel-child job-a agent-a"),
        CodingCommand::CancelChild {
            id: "job-a".into(),
            agent: "agent-a".into(),
        }
    );
}
