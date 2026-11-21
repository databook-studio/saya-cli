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

/// The one axis on the launch: `--mode plan` states plan, through the same
/// grammar and the same gate `/mode` reads, and owes no deprecation note.
#[test]
fn the_mode_flag_states_plan_and_owes_no_note() {
    use crate::interactive::session::permissions::mode::stated_mode;

    let cli = Cli::try_parse_from(["saya", "--mode", "plan"]).expect("`--mode plan` parses");
    assert!(cli.command.is_none());
    assert_eq!(cli.options.mode.as_deref(), Some("plan"));
    assert_eq!(
        stated_mode("--mode", cli.options.mode.as_deref().unwrap(), false),
        Ok(saya_agent::PermissionMode::Plan),
        "the flag resolves through the one six-value grammar"
    );
    assert_eq!(consent_alias_note(&cli.options), None, "no alias, no note");
}

/// `--approval-mode` still works for one release, and says on stderr that it is
/// the alias: the posture it states is the same one the axis states.
#[test]
fn the_approval_mode_alias_still_works_and_notes_itself() {
    use crate::interactive::session::permissions::mode::APPROVAL_MODE_FLAG_NOTE;

    let cli =
        Cli::try_parse_from(["saya", "--approval-mode", "read-only"]).expect("the alias parses");
    assert_eq!(
        config::runtime::approval_mode(&cli.options).expect("the alias resolves"),
        saya_agent::ApprovalPolicy::ReadOnly,
        "the alias states the same posture the axis does"
    );
    assert_eq!(
        consent_alias_note(&cli.options),
        Some(APPROVAL_MODE_FLAG_NOTE),
        "the alias says it is an alias, once"
    );

    let cli =
        Cli::try_parse_from(["saya", "--permission-mode", "never"]).expect("the alias parses");
    assert_eq!(
        consent_alias_note(&cli.options),
        Some(crate::interactive::session::permissions::mode::PERMISSION_MODE_FLAG_NOTE)
    );
}

/// Stating two of the three consent flags is refused rather than resolved by
/// precedence — `--mode` beside either alias included.
#[test]
fn two_consent_flags_on_one_launch_are_refused() {
    for argv in [
        &["saya", "--approval-mode", "ask", "--mode", "never"][..],
        &["saya", "--mode", "never", "--permission-mode", "ask"][..],
        &[
            "saya",
            "--permission-mode",
            "plan",
            "--approval-mode",
            "read-only",
        ][..],
    ] {
        let cli = Cli::try_parse_from(argv).expect("each flag parses on its own");
        let error = refuse_conflicting_consent(
            cli.options.mode.as_deref(),
            cli.options.permission_mode.as_deref(),
            cli.options.approval_mode.as_deref(),
        )
        .expect_err("two spellings of one consent must refuse");
        assert!(
            error.contains("both state the session's consent"),
            "the refusal keeps today's wording style: {error}"
        );
    }
    let cli = Cli::try_parse_from(["saya", "--mode", "never"]).expect("one flag parses");
    assert!(
        refuse_conflicting_consent(
            cli.options.mode.as_deref(),
            cli.options.permission_mode.as_deref(),
            cli.options.approval_mode.as_deref(),
        )
        .is_ok(),
        "one stated consent is never a conflict"
    );
}
