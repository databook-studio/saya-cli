//! The `/run` child: one nested `saya run` invocation, streamed unmangled.
//!
//! The session's `/run <tail…>` starts a headless run by spawning the real
//! `saya run` command as a child process and handing it the slash tail after
//! its small no-shell argv tokenizer. The child's own CLI parser stays the
//! authority on `--allow`, `--budget`, and the `resume`/`show`/`log`/`list`
//! subcommands — the slash adapter parses no command grammar twice, so the two
//! surfaces cannot drift.
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
use crate::interactive::session::state::SessionState;
use crate::render::RenderFormat;
use clap::Parser as _;
use std::process::{Command, Stdio};

/// The run subcommands the child's CLI recognizes after `run`. A tail whose
/// first token is one of these passes through verbatim (subcommand form);
/// anything else is the goal.
const SUBCOMMAND_WORDS: [&str; 5] = ["cancel", "resume", "show", "log", "list"];

/// The slash adapter's own word: `/run --seed-grants <tail…>` seeds the
/// child's `--allow` from this session's grants on request. The flag is the
/// tail's first token and never reaches the child; anything else passes
/// through verbatim, and the child's own parser refuses it as the unknown
/// flag it is — the adapter parses one word, the child's parser stays the
/// authority on the rest.
pub(crate) fn separate_seed_flag(tail: &str) -> (bool, &str) {
    let Some(rest) = tail.strip_prefix("--seed-grants") else {
        return (false, tail);
    };
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        // A longer flag that merely starts with the word is the child's
        // business, not a seed request.
        return (false, tail);
    }
    (true, rest.trim_start())
}

include!("launch.rs");

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
    Resume {
        run_id: String,
        retry_incomplete: bool,
    },
}

/// Parses a `/run <tail>` tail through the child's own grammar. The TUI
/// routes `Start` tails to the run panel and `Manage` tails through the
/// shared dispatcher; `resume` is declined (see [`RunTail::Resume`]).
pub(crate) fn parse_run_tail(tail: &str) -> Result<RunTail, String> {
    let mut argv = vec!["saya".to_string(), "--non-interactive".to_string()];
    argv.push("run".to_string());
    argv.extend(child_argv(tail)?);
    match crate::cli::Cli::try_parse_from(argv) {
        Ok(cli) => match cli.command {
            Some(crate::cli::Command::Run {
                prompt,
                allow,
                budget,
                command,
            }) => match command {
                Some(crate::cli::RunCommand::Resume {
                    run_id,
                    retry_incomplete,
                }) => Ok(RunTail::Resume {
                    run_id,
                    retry_incomplete,
                }),
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
#[path = "tests.rs"]
mod tests;
