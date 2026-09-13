//! The `/run` child: one nested `saya run` invocation, streamed unmangled.
//!
//! The session's `/run <tail…>` starts a headless run by spawning the real
//! `saya run` command as a child process and handing it the raw tail
//! verbatim. The child's own CLI parser stays the authority on `--allow`,
//! `--budget`, and the `resume`/`show`/`log`/`list` subcommands — the slash
//! adapter parses nothing twice, so the two surfaces cannot drift.
//!
//! **The dual-tag hazard, and what this module does about it.** A nested
//! `saya` emits its own event stream — `TerminalEvent` lines tagged
//! `"event"` and `RunEvent` lines tagged `"type"` — inside the parent's
//! stdout. When the parent session is itself on a wire (`--format ndjson`),
//! that puts a complete child stream inside a parent stream. The parent must
//! neither re-tag the child's lines (wrapping them in the parent's envelope
//! would change bytes a harness has already learned to parse) nor swallow
//! them (a parent-side filter that drops what it does not recognize would
//! lose the run's record). The answer here is to pass the file descriptors
//! through untouched: the child's stdout and stderr are `Stdio::inherit`, so
//! its bytes land on the parent's real stdout byte-for-byte, unobserved by
//! the parent's renderer. The parent's own lines simply continue when the
//! child exits. The cost is deliberate: the parent cannot decorate, filter,
//! or re-render a nested run — a run's stream is the run's own.
//!
//! The child reads the same process env (config home, state database, runs
//! root, provider settings), so `/runs` in the session and the child's
//! journal describe the same runs on disk. Explicit `--config` /
//! `--connections` overrides and the session's active profile and approval
//! mode are forwarded so the child resolves what the session resolved.

use crate::config::runtime::RuntimeConfig;
use crate::interactive::session_state::SessionState;
use crate::render::RenderFormat;
use clap::Parser as _;
use std::process::{Command, Stdio};

/// The run subcommands the child's CLI recognizes after `run`. A tail whose
/// first token is one of these passes through verbatim (subcommand form);
/// anything else is the goal.
const SUBCOMMAND_WORDS: [&str; 5] = ["cancel", "resume", "show", "log", "list"];

/// Spawns the nested `saya run <tail…>` child, streams its output through
/// unmangled, and waits for it. The child's exit code is its own: it already
/// said why the run ended (its settle message names completed, paused, the
/// resume hint, or the failure), and the parent adds nothing to the stream —
/// any echo here would be exactly the re-tagging the module doc refuses.
pub(crate) fn spawn_run_child(
    runtime: &RuntimeConfig,
    format: RenderFormat,
    state: &SessionState,
    tail: &str,
) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let mut command = Command::new(exe);
    command
        .arg("--non-interactive")
        .arg("--format")
        .arg(format_flag(format))
        .arg("run")
        .args(child_argv(tail));
    // Forward the config sources the session actually loaded, so the child
    // resolves the same profiles, provider, and secrets — a run must not
    // silently run against a different configuration than the session's.
    if let Some(config) = &runtime.config_path {
        command.arg("--config").arg(config);
    }
    if let Some(connections) = &runtime.connections_path {
        command.arg("--connections").arg(connections);
    }
    if let Some(profile) = state.profile.as_deref() {
        command.arg("--profile").arg(profile);
    }
    // Forward the session's approval mode so the child resolves what the
    // session resolved — except bypass: a run's approval is its `--allow`
    // scopes (`commands/run/start.rs`), so a bypass session's blanket
    // per-call consent never reaches the child, which takes its own default
    // (read-only). The session's bypass never claimed to reach the child;
    // the run states its own scopes.
    if let Some(mode) = forwarded_approval_mode(state.approval_mode.as_str()) {
        command.arg("--approval-mode").arg(mode);
    }
    // The child never reads stdin (a headless run prompts for nothing), and
    // it must never consume the session's remaining input lines: null stdin.
    command
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    command.status().map(|_| ())
}

/// Shapes the child's `run` argument vector from the raw slash tail:
/// a subcommand tail passes through verbatim (the child's parser reads its
/// id), and a goal tail rejoins its leading words into the single positional
/// the CLI declares — a goal is one string, then the flags verbatim.
fn child_argv(tail: &str) -> Vec<String> {
    let tokens = tail.split_whitespace().collect::<Vec<_>>();
    if matches!(tokens.first(), Some(first) if SUBCOMMAND_WORDS.contains(first)) {
        return tokens.into_iter().map(str::to_string).collect();
    }
    let boundary = tokens
        .iter()
        .position(|token| token.starts_with("--"))
        .unwrap_or(tokens.len());
    let mut argv = vec![tokens[..boundary].join(" ")];
    argv.extend(tokens[boundary..].iter().map(|token| token.to_string()));
    argv
}

/// The approval mode forwarded to a nested `saya run` child: the session's
/// mode verbatim — except bypass. A run's approval is its `--allow` scopes;
/// a bypass session's blanket consent never reaches the child, which states
/// its own scopes or takes the run default (`app.rs`). `None` forwards
/// nothing.
fn forwarded_approval_mode(mode: &str) -> Option<&str> {
    (mode != "bypass").then_some(mode)
}

/// The format flag the child inherits, so a piped session's `/run` speaks the
/// same wire the session itself speaks.
fn format_flag(format: RenderFormat) -> &'static str {
    match format {
        RenderFormat::Text => "text",
        RenderFormat::Json => "json",
        RenderFormat::Ndjson => "ndjson",
    }
}

/// What a `/run <tail>` tail means once the child's own grammar has parsed
/// it. The same clap grammar `saya run` uses parses the tail in-process, so
/// the TUI's panel adapter parses nothing twice — the parser stays the
/// authority on `--allow`, `--budget`, and the subcommands.
#[derive(Debug)]
pub(crate) enum RunTail {
    /// A fresh run: goal, scopes, and budgets. The run panel drives it.
    Start {
        goal: Option<String>,
        allow: Vec<String>,
        budget: Vec<String>,
    },
    /// A management subcommand (`show`/`log`/`list`/`cancel`) — the shared
    /// dispatcher handles it, the same path `saya run` and `/runs` take.
    Manage(crate::cli::RunCommand),
    /// `resume` — the resume drive streams to the real stdout, which the TUI
    /// does not own while the alternate screen is up; a shell hosts it.
    Resume(String),
}

/// Parses a `/run <tail>` tail through the child's own grammar. The TUI
/// routes `Start` tails to the run panel and `Manage` tails through the
/// shared dispatcher; `resume` is declined (see [`RunTail::Resume`]).
pub(crate) fn parse_run_tail(tail: &str) -> Result<RunTail, String> {
    let mut argv = vec!["saya".to_string(), "--non-interactive".to_string()];
    argv.push("run".to_string());
    argv.extend(child_argv(tail));
    match crate::cli::Cli::try_parse_from(argv) {
        Ok(cli) => match cli.command {
            Some(crate::cli::Command::Run {
                prompt,
                allow,
                budget,
                command,
            }) => match command {
                Some(crate::cli::RunCommand::Resume { run_id }) => Ok(RunTail::Resume(run_id)),
                Some(other) => Ok(RunTail::Manage(other)),
                None => Ok(RunTail::Start {
                    goal: prompt,
                    allow,
                    budget,
                }),
            },
            _ => Err("expected a run command: /run <goal> --allow <scopes>".to_string()),
        },
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A goal tail rejoins its words into the single positional the headless
    /// CLI declares, and passes the flags verbatim — the child's parser stays
    /// the authority on them.
    #[test]
    fn a_goal_tail_becomes_one_positional_then_flags() {
        assert_eq!(
            child_argv("survey the data --allow workspace-write"),
            ["survey the data", "--allow", "workspace-write"]
        );
        assert_eq!(child_argv("one goal"), ["one goal"]);
        // No goal words at all: an empty positional, which the child's own
        // parser refuses — the adapter does not pre-validate what the child
        // rejects.
        assert_eq!(
            child_argv("--allow workspace-write"),
            ["", "--allow", "workspace-write"]
        );
    }

    /// A subcommand tail passes through verbatim, word by word, so
    /// `/run resume <id>` and `/run list` reach the child exactly as typed.
    #[test]
    fn subcommand_tails_pass_through_verbatim() {
        assert_eq!(child_argv("resume r-1"), ["resume", "r-1"]);
        assert_eq!(child_argv("list"), ["list"]);
    }

    /// The panel path parses the tail through the same grammar the child
    /// gets: a goal tail becomes one positional plus flags, a management
    /// subcommand maps to the shared `RunCommand`, and `resume` is surfaced
    /// as its own decline (the resume drive streams to the real stdout).
    #[test]
    fn the_panel_parses_the_tail_through_the_child_grammar() {
        match parse_run_tail("survey the data --allow workspace-write") {
            Ok(RunTail::Start {
                goal,
                allow,
                budget,
            }) => {
                assert_eq!(goal.as_deref(), Some("survey the data"));
                assert_eq!(allow, vec!["workspace-write".to_string()]);
                assert!(budget.is_empty());
            }
            other => panic!("a goal tail parses to Start, got {other:?}"),
        }
        match parse_run_tail("show r-1") {
            Ok(RunTail::Manage(crate::cli::RunCommand::Show { run_id })) => {
                assert_eq!(run_id, "r-1");
            }
            other => panic!("a show tail parses to Manage, got {other:?}"),
        }
        match parse_run_tail("log r-1") {
            Ok(RunTail::Manage(crate::cli::RunCommand::Log { run_id })) => {
                assert_eq!(run_id, "r-1");
            }
            other => panic!("a log tail parses to Manage, got {other:?}"),
        }
        match parse_run_tail("resume r-1") {
            Ok(RunTail::Resume(run_id)) => assert_eq!(run_id, "r-1"),
            other => panic!("a resume tail parses to Resume, got {other:?}"),
        }
        // The parser is the authority: an unknown flag fails the way the
        // child's own parse would. (A budget *value* is validated later, by
        // the run surface — the grammar itself accepts it.)
        assert!(parse_run_tail("--nonsense").is_err());
        match parse_run_tail("--budget nonsense") {
            Ok(RunTail::Start { budget, .. }) => {
                assert_eq!(budget, vec!["nonsense".to_string()]);
            }
            other => panic!("a budget tail parses to Start, got {other:?}"),
        }
    }

    /// A bypass session's nested `saya run` child gets no `--approval-mode`
    /// flag: a run's approval is its `--allow` scopes, and a bypass session's
    /// blanket per-call consent never claimed to reach the child. Every other
    /// mode is forwarded verbatim, as before.
    #[test]
    fn a_bypass_session_s_nested_run_child_gets_no_bypass_mode() {
        assert_eq!(forwarded_approval_mode("bypass"), None);
        for (mode, expected) in [
            ("ask", Some("ask")),
            ("read-only", Some("read-only")),
            ("never", Some("never")),
        ] {
            assert_eq!(
                forwarded_approval_mode(mode),
                expected,
                "a {mode} session's child resolves what the session resolved"
            );
        }
        // The guard is the vocabulary's own word, not a prefix match: a mode
        // that merely contains "bypass" is not bypass.
        assert!(
            forwarded_approval_mode("bypassish").is_some(),
            "an unknown mode word is forwarded verbatim, never treated as bypass"
        );
    }
}
