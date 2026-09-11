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
    command
        .arg("--approval-mode")
        .arg(state.approval_mode.as_str());
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

/// The format flag the child inherits, so a piped session's `/run` speaks the
/// same wire the session itself speaks.
fn format_flag(format: RenderFormat) -> &'static str {
    match format {
        RenderFormat::Text => "text",
        RenderFormat::Json => "json",
        RenderFormat::Ndjson => "ndjson",
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
}
