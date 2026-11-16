//! The `[hooks]` table: user-configured commands that run at agent lifecycle
//! points.
//!
//! Six events, one table each: `[[hooks.pre_tool_use]]` runs before a tool
//! call and may **block** it, `[[hooks.post_tool_use]]` runs after a call and
//! only observes, `[[hooks.stop]]` runs when the agent finishes a turn,
//! `[[hooks.session_start]]` and `[[hooks.session_end]]` bracket an interactive
//! session or a job root, and `[[hooks.permission_request]]` runs when a call
//! would ask the operator and may refuse it.
//!
//! Every hook is the user's own command. The table is protected whole, so a
//! repository's `.saya/config.toml` cannot state it (see
//! `layers::protected`): a hook runs on the host lane, and a model-writable
//! file must not be able to add one. A hook can only ever *narrow*: a
//! PreToolUse exit code refuses a call the surface already allowed, and no
//! exit code can grant a tool, widen a scope, change an approval mode, or
//! answer an ask on the user's behalf.
//!
//! [`ResolvedHooks`] holds the validated form — every bound applied at resolve
//! time so the engine never has to re-check a command it is about to run.

mod merge;
mod resolve;

pub use resolve::{HookEvent, ResolvedHook, ResolvedHookCommand, ResolvedHooks};

pub(crate) use merge::merge;
pub(crate) use resolve::resolve;

use serde::Deserialize;

/// How many hooks one event may hold. Each one is a process, and the events
/// that carry them sit on the agent loop's critical path, so the count is a
/// bound on the loop's per-call cost — not a policy the user can widen by
/// writing more entries.
pub const MAX_HOOKS_PER_EVENT: usize = 8;

/// The command's byte ceiling, the same one a `shell` call's command carries:
/// a hook is a shell command, and the two must not disagree about what fits.
pub const MAX_HOOK_COMMAND_BYTES: usize = 8192;

/// The argv form's two bounds: how many words, and how many bytes each.
pub const MAX_HOOK_ARGV: usize = 64;
pub const MAX_HOOK_ARG_BYTES: usize = 1024;

/// A hook's timeout when its entry names none, and the ceiling an entry may
/// not exceed. The default is far below a `shell` call's, because a hook runs
/// on the agent loop's own path: a hook that takes ten seconds has already
/// cost the turn more than the tool it was watching.
pub const DEFAULT_HOOK_TIMEOUT_MS: u64 = 10_000;
pub const MAX_HOOK_TIMEOUT_MS: u64 = 60_000;

/// Raw trusted `[hooks]` settings. Every list stays optional until layers
/// merge, so a higher-precedence layer can override one event at a time.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HooksFile {
    /// Runs before a tool call. Exit 2 blocks the call; every other exit
    /// allows it.
    #[serde(default)]
    pub pre_tool_use: Vec<HookEntryFile>,
    /// Runs after a tool call. Observes only — its exit code is a diagnostic,
    /// never a decision.
    #[serde(default)]
    pub post_tool_use: Vec<HookEntryFile>,
    /// Runs when the agent finishes a turn. Observes only.
    #[serde(default)]
    pub stop: Vec<HookEntryFile>,
    /// Runs once when an interactive session or a job root starts. Observes
    /// only; its stdout is offered to the model as additional context.
    #[serde(default)]
    pub session_start: Vec<HookEntryFile>,
    /// Runs when the session or job root ends, normal or cancelled. Observes
    /// only.
    #[serde(default)]
    pub session_end: Vec<HookEntryFile>,
    /// Runs when a call would ask the operator. Exit 2 denies the call; exit
    /// 0 falls through to the normal ask.
    #[serde(default)]
    pub permission_request: Vec<HookEntryFile>,
}

/// One `[[hooks.<event>]]` entry.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HookEntryFile {
    /// The tool name this entry watches. Absent or `"*"` matches every tool.
    /// An exact name only: a hook is a command, not a pattern language, and a
    /// matcher that read wider than it looked would run a user's command on
    /// calls they did not mean to watch.
    #[serde(default)]
    pub matcher: Option<String>,
    /// The command: a shell string, or argv. Exactly one shape.
    pub command: HookCommandFile,
    /// The entry's timeout in milliseconds. Absent means
    /// [`DEFAULT_HOOK_TIMEOUT_MS`].
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// The two shapes a hook command may take. A shell string is handed to
/// `/bin/sh -c` exactly as a `shell` call's command is; an argv list is
/// quoted into the same command line so the shell interprets nothing inside
/// the arguments.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum HookCommandFile {
    Shell(String),
    Argv(Vec<String>),
}
