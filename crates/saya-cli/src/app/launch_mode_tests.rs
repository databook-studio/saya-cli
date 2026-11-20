//! `--permission-mode` on a launch surface that refuses it: the interactive
//! session is the only reader, so every subcommand answers the flag with the
//! same sentence — a subcommand that quietly ignored a stated consent would be
//! the worse answer.

use super::*;
use clap::Parser as _;

/// Today's exact refusal. Spelled out here rather than read from the guard, so
/// a reworded or reflowed sentence turns this red.
const REFUSAL: &str = "`--permission-mode` starts the interactive session in a permission \
                       mode, not a subcommand: launch the session (`saya --permission-mode \
                       <mode>`) or run the subcommand without it";

/// The refusal `argv` earns: the flag parses before the subcommand, and the
/// subcommand's guard is what answers it.
fn refused(argv: &[&str]) -> String {
    let cli = Cli::try_parse_from(argv).expect("the flag parses before a subcommand");
    assert!(cli.command.is_some(), "{argv:?} names a subcommand");
    assert_eq!(
        cli.options.permission_mode.as_deref(),
        Some("bypass"),
        "{argv:?} carries the stated mode"
    );
    refuse_permission_mode_on_subcommand(cli.options.permission_mode.as_deref())
        .expect_err("a subcommand must refuse a stated permission mode")
}

/// Every launch surface that takes a mode today — `start`, `job resume`,
/// `follow-up`, `mcp serve` — refuses `bypass` in the one wording, whatever
/// the mode's value. The mode stated here is the widest one, because a
/// refusal that changed with the mode would be a second policy.
#[test]
fn every_subcommand_refuses_a_stated_bypass_mode_in_the_same_words() {
    for argv in [
        &[
            "saya",
            "--permission-mode",
            "bypass",
            "start",
            "job-a",
            "a task",
            "--approve",
        ][..],
        &[
            "saya",
            "--permission-mode",
            "bypass",
            "job",
            "resume",
            "job-a",
            "--approve",
        ][..],
        &[
            "saya",
            "--permission-mode",
            "bypass",
            "follow-up",
            "job-a",
            "agent-a",
            "--expected-completion-sequence",
            "1",
            "go",
            "--approve",
        ][..],
        &["saya", "--permission-mode", "bypass", "mcp", "serve"][..],
    ] {
        assert_eq!(refused(argv), REFUSAL, "{argv:?}");
    }
}

/// The post-subcommand spelling never reaches the guard: the flag is not
/// global, so clap refuses it as an unknown argument rather than a subcommand
/// silently dropping it.
#[test]
fn a_mode_stated_after_the_subcommand_never_parses() {
    for argv in [
        &[
            "saya",
            "start",
            "job-a",
            "a task",
            "--approve",
            "--permission-mode",
            "bypass",
        ][..],
        &[
            "saya",
            "job",
            "resume",
            "job-a",
            "--approve",
            "--permission-mode",
            "bypass",
        ][..],
    ] {
        let error = Cli::try_parse_from(argv).expect_err("the flag is not global");
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::UnknownArgument,
            "{argv:?}"
        );
    }
}

/// `-p` is the one launch that states a mode itself, and it accepts `bypass`:
/// the flag parses beside a print task and names no subcommand, so the guard
/// that refuses it on a subcommand is never reached. Whether `-p` accepts the
/// mode at all is the one-shot plan's own contract, pinned in `oneshot_tests`.
#[test]
fn the_print_launch_still_takes_bypass() {
    let cli = Cli::try_parse_from(["saya", "-p", "fix the test", "--permission-mode", "bypass"])
        .expect("`-p` takes the flag");
    assert!(cli.command.is_none(), "`-p` is a whole launch");
    assert!(cli.print.task.is_some());
    assert_eq!(cli.options.permission_mode.as_deref(), Some("bypass"));
}
