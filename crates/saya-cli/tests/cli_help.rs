//! Regression guard for the `--help` surface (S14). A user who runs `saya
//! --help` decides what to do next from the one-line summaries and the per-flag
//! descriptions, so a blank entry is a silent hole — it reads as "trivial" or
//! "undocumented" with no way to tell which. This test walks clap's command tree
//! (root → every subcommand, at every depth) and fails on any subcommand with no
//! `about`, or any argument with no `help`, naming the offender. It is the only
//! thing that keeps a future addition from reintroducing a blank.
//!
//! Introspection uses the same `CommandFactory::command()` entry point the rest
//! of the test suite uses. clap sets `about` from the first doc-comment line
//! and `help` from an arg's `///`, so a missing doc comment shows up as `None`.

use clap::{CommandFactory, Parser};
use saya_cli::Cli;

/// The arg ids clap synthesizes for its own `help` and `version` flags. They
/// carry built-in help text ("Print help" / "Print version"), so they never
/// regress, and they are not ours to document — skip them.
const CLAP_INTERNAL_ARGS: &[&str] = &["help", "version"];

#[test]
fn every_subcommand_and_argument_has_help_text() {
    let root = Cli::command();
    let mut offenders: Vec<String> = Vec::new();
    walk(&root, &mut offenders);

    if !offenders.is_empty() {
        panic!(
            "blank --help entries (no description found):\n  - {}\n\
             Add a `///` doc comment (subcommands) or `///` on the arg field \
             (arguments) so `--help` is never silent.",
            offenders.join("\n  - ")
        );
    }
}

/// Depth-first: check `cmd` itself, then every argument it declares, then
/// recurse into its subcommands. The root is included, so a missing top-level
/// `about` is caught too.
fn walk(cmd: &clap::Command, offenders: &mut Vec<String>) {
    let path = command_path(cmd);

    if cmd
        .get_about()
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty())
        .is_none()
    {
        offenders.push(format!("subcommand `{path}` has no description"));
    }

    for arg in cmd.get_arguments() {
        let id = arg.get_id().as_str().to_string();
        if CLAP_INTERNAL_ARGS.contains(&id.as_str()) {
            continue;
        }
        let has_help = arg
            .get_help()
            .map(|s| s.to_string())
            .filter(|s| !s.trim().is_empty())
            .is_some();
        if !has_help {
            let kind = if arg.is_positional() {
                "positional"
            } else {
                "flag"
            };
            offenders.push(format!("`{path}` {kind} `{id}` has no description"));
        }
    }

    for sub in cmd.get_subcommands() {
        walk(sub, offenders);
    }
}

/// Returns the command's leaf name so a failure points at the exact surface,
/// not an anonymous `Command`. clap does not expose parent links on the built
/// tree, so a dotted path (`saya contracts decide`) is not available — but
/// saya's tree uses unique leaf names, so the leaf alone locates the entry.
fn command_path(cmd: &clap::Command) -> String {
    cmd.get_name().to_string()
}

// ---------------------------------------------------------------------------
// S15 — `config show` advertised two flags it never read (S15 spec, invariants
// 1 and 4). Both `--resolved` and `--redacted` were accepted and discarded
// since the initial release: `config show` always printed the resolved,
// redacted view regardless. Keeping a flag that implies redaction is optional
// is worse than no flag (invariant 2 — redaction is never optional), so the
// slice removes both. Verified against the unchanged binary beforehand:
// `config show`, `config show --resolved`, `config show --redacted`, and
// `config show --resolved --redacted` produced byte-identical output (run with
// `--config /dev/null --connections /dev/null` for a hermetic input), and
// `config show --help` advertised both flags. The two tests below pin the
// post-removal surface: the flags no longer parse and are no longer advertised.
// ---------------------------------------------------------------------------

/// `config show --resolved` and `config show --redacted` no longer parse: a
/// script passing the dead flags now gets a clap error instead of a silent
/// success that changed nothing. This is the breaking change recorded in the
/// changelog — the flags never had an effect, so the output is identical to
/// plain `config show` for anyone who drops them.
#[test]
fn config_show_no_longer_accepts_resolved_or_redacted() {
    for flag in ["--resolved", "--redacted"] {
        let parsed = Cli::try_parse_from(["saya", "config", "show", flag]);
        assert!(
            parsed.is_err(),
            "`config show {flag}` must not parse after S15: {parsed:?}"
        );
    }
}

/// `config show --help` no longer advertises the removed flags, so the help
/// surface and behaviour agree (invariant 1). A user reading `--help` should
/// not find an off switch for redaction that does not exist.
#[test]
fn config_show_help_no_longer_advertises_resolved_or_redacted() {
    let mut cmd = Cli::command();
    let show_help = cmd
        .find_subcommand_mut("config")
        .expect("`config` subcommand exists")
        .find_subcommand_mut("show")
        .expect("`config show` subcommand exists")
        .render_help()
        .to_string();
    for flag in ["--resolved", "--redacted"] {
        assert!(
            !show_help.contains(flag),
            "`config show --help` must not advertise {flag} after S15"
        );
    }
}

/// S28 deliverable 1: `contracts approve-all --help` states what the batch
/// approves, that every item still gets the per-item validation (so some are
/// refused and every refusal is reported), and that consent is explicit via
/// `--yes`. A user deciding whether to run it must be able to learn from the
/// help alone that approving is preview-then-consent, never silent.
#[test]
fn approve_all_help_states_scope_consent_and_per_item_reporting() {
    let mut cmd = Cli::command();
    let help = cmd
        .find_subcommand_mut("contracts")
        .expect("`contracts` subcommand exists")
        .find_subcommand_mut("approve-all")
        .expect("`contracts approve-all` subcommand exists")
        .render_help()
        .to_string();
    // The scope: the bounded queue the user was shown, not the archive.
    assert!(
        help.contains("review queue"),
        "the help names the queue scope: {help}"
    );
    // The failure mode this command exists to prevent: refusals are reported,
    // not aggregated away.
    assert!(
        help.contains("refused"),
        "the help says refusals are reported: {help}"
    );
    // Consent is explicit and visible before anything happens.
    assert!(
        help.contains("--yes"),
        "the help documents the --yes consent flag: {help}"
    );

    // And the surface parses: scope (Q1) is the optional --profile, the queue
    // bound (Q3) is the optional --limit, consent is --yes.
    let parsed = Cli::try_parse_from([
        "saya",
        "contracts",
        "approve-all",
        "--yes",
        "--limit",
        "5",
        "--profile",
        "local",
    ])
    .expect("approve-all parses profile/limit/yes");
    match parsed.command {
        Some(saya_cli::Command::Contracts {
            command:
                saya_cli::ContractsCommand::ApproveAll {
                    profile,
                    limit,
                    yes,
                },
        }) => {
            assert_eq!(profile.as_deref(), Some("local"));
            assert_eq!(limit, Some(5));
            assert!(yes);
        }
        other => panic!("expected Contracts ApproveAll, got {other:?}"),
    }
}
