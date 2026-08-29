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

use clap::CommandFactory;
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
