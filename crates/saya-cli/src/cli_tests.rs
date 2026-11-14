//! Unit tests for `cli.rs` types: the Debug impls the adapters embed in
//! diagnostics must not carry user-supplied values, and the `-p` guards
//! refuse a stated intent rather than silently dropping it.

use crate::cli::{Cli, InvestigationCommand};
use clap::Parser as _;

/// The run command's Debug names its `--param` bindings and never prints a
/// value: the TUI replay task and the session command enum embed the
/// command in their own Debug, so a bound value must not ride diagnostics
/// with it. A malformed entry with no `=` is fully redacted, and the other
/// fields still print as the derive would.
#[test]
fn the_run_command_debug_names_params_without_values() {
    let command = InvestigationCommand::Run {
        id: "inv-run-1".to_owned(),
        connection: Some("local".to_owned()),
        revalidate: false,
        report: None,
        rows: None,
        overwrite: false,
        params: vec![
            "token=sk-live-abc123".to_owned(),
            "label=a=b".to_owned(),
            "malformed".to_owned(),
        ],
    };
    let rendered = format!("{command:?}");
    assert!(
        rendered.contains("token=…") && rendered.contains("label=…"),
        "each bound name prints with a value marker: {rendered}"
    );
    assert!(
        !rendered.contains("sk-live-abc123") && !rendered.contains("a=b"),
        "no value or value fragment prints: {rendered}"
    );
    assert!(
        !rendered.contains("malformed"),
        "an entry without a name is fully redacted: {rendered}"
    );
    assert!(
        rendered.contains("local"),
        "the other fields still print: {rendered}"
    );
}

/// `--tools` has exactly one declaration: the global flag. `saya start`
/// carries no copy of its own, so the pre-subcommand spelling and the
/// post-subcommand one state the same one list — there is no local copy to
/// shadow the global value, and no second help text.
#[test]
fn print_with_a_subcommand_is_refused() {
    let cli = Cli::try_parse_from(["saya", "-p", "a task", "ask", "a question"])
        .expect("the flags parse before the subcommand");
    assert!(cli.command.is_some());
    let error = super::print::refuse_subcommand(cli.command.as_ref(), cli.print.task.as_deref())
        .expect_err("`-p` beside a subcommand must refuse");
    assert!(
        error.contains("takes no subcommand"),
        "the refusal names the conflict: {error}"
    );
    assert!(
        super::print::refuse_subcommand(None, Some("a task")).is_ok(),
        "no subcommand: the one-shot surface is the launch"
    );
}

/// `--continue` and `--resume` name an interactive session; `-p` resumes
/// nothing and points at the job verb instead.
#[test]
fn print_refuses_continue_and_resume() {
    for argv in [
        vec!["saya", "--continue", "-p", "a task"],
        vec!["saya", "--resume", "session-1", "-p", "a task"],
    ] {
        let cli = Cli::try_parse_from(argv).expect("the flag parses");
        let error = super::print::refuse_session_flags(&cli.options)
            .expect_err("a session flag must refuse on `-p`");
        assert!(
            error.contains("saya job resume"),
            "the refusal names the job verb: {error}"
        );
    }
}

/// `--allow`/`--deny` are session grants; `-p` has no session. The refusal
/// names the command-rule spelling that does work on `-p`.
#[test]
fn print_refuses_session_allow_and_names_the_command_rule() {
    let cli =
        Cli::try_parse_from(["saya", "--allow", "shell", "-p", "a task"]).expect("the flag parses");
    let error = super::print::refuse_session_flags(&cli.options)
        .expect_err("a session grant must refuse on `-p`");
    assert!(
        error.contains("--allow-command"),
        "the refusal names the spelling that works: {error}"
    );

    let cli =
        Cli::try_parse_from(["saya", "--deny", "rm", "-p", "a task"]).expect("the flag parses");
    let error = super::print::refuse_session_flags(&cli.options)
        .expect_err("a session deny must refuse on `-p`");
    assert!(
        error.contains("--deny-command"),
        "the refusal names the spelling that works: {error}"
    );

    // The command-rule flags are accepted: P6 rules apply to `-p`.
    let cli = Cli::try_parse_from([
        "saya",
        "--allow-command",
        "cargo test",
        "--deny-command",
        "rm",
        "-p",
        "a task",
    ])
    .expect("the command-rule flags parse");
    assert!(
        super::print::refuse_session_flags(&cli.options).is_ok(),
        "command rules are not session grants"
    );
}

/// `--workspace` and `--turn-file` bind an interactive session; `-p` always
/// runs in the current directory and takes its task from `-p`.
#[test]
fn print_refuses_the_other_session_only_flags() {
    for (argv, needle) in [
        (
            vec!["saya", "--workspace", "/tmp/p", "-p", "a task"],
            "--workspace",
        ),
        (
            vec!["saya", "--turn-file", "/tmp/t", "-p", "a task"],
            "--turn-file",
        ),
    ] {
        let cli = Cli::try_parse_from(argv).expect("the flag parses");
        let error = super::print::refuse_session_flags(&cli.options)
            .expect_err("a session-only flag must refuse on `-p`");
        assert!(
            error.contains(needle),
            "the refusal names {needle}: {error}"
        );
    }
}

/// `--max-turns` and the global logical-request limit are one ceiling, and a
/// budget must be a finite, non-negative amount.
#[test]
fn print_refuses_conflicting_or_invalid_limits() {
    let cli = Cli::try_parse_from([
        "saya",
        "--max-turns",
        "2",
        "--coding-max-logical-model-requests",
        "3",
        "-p",
        "a task",
    ])
    .expect("both flags parse");
    let error = super::print::refuse_conflicting_limits(&cli.print, &cli.options)
        .expect_err("two spellings of one limit must refuse");
    assert!(
        error.contains("--max-turns") && error.contains("--coding-max-logical-model-requests"),
        "the refusal names both: {error}"
    );

    let cli = Cli::try_parse_from(["saya", "--max-budget-usd=-1", "-p", "a task"])
        .expect("the flag parses");
    let error = super::print::refuse_conflicting_limits(&cli.print, &cli.options)
        .expect_err("a negative budget must refuse");
    assert!(error.contains("non-negative"), "{error}");
}

#[test]
fn the_tools_flag_has_one_declaration_for_start() {
    use clap::Parser;

    for argv in [
        [
            "saya",
            "start",
            "job-a",
            "a task",
            "--tools",
            "read",
            "--approve",
        ],
        [
            "saya",
            "--tools",
            "read",
            "start",
            "job-a",
            "a task",
            "--approve",
        ],
    ] {
        let cli = super::Cli::try_parse_from(argv).expect("the flag parses in either position");
        assert_eq!(
            cli.options.tools,
            ["read"],
            "{argv:?} states the one global list"
        );
        assert!(
            matches!(cli.command, Some(super::Command::Start(_))),
            "{argv:?} parses a start"
        );
    }
}

#[test]
fn run_resume_retry_incomplete_is_an_explicit_cli_input() {
    use clap::Parser;

    let cli = super::Cli::try_parse_from(["saya", "run", "resume", "r-test", "--retry-incomplete"])
        .expect("the operator retry flag parses on the real run-resume command");

    assert!(matches!(
        cli.command,
        Some(super::Command::Run {
            command: Some(super::RunCommand::Resume {
                run_id,
                retry_incomplete: true,
            }),
            ..
        }) if run_id == "r-test"
    ));
}

#[test]
fn print_refuses_a_blank_stated_task() {
    let error = super::print::task_text("   \n\t").expect_err("a whitespace-only task must refuse");
    assert!(error.contains("needs a task"), "{error}");
}
