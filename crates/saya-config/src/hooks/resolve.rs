//! Turning the raw `[hooks]` table into the validated form the engine runs:
//! every bound applied once, here, with a typed error naming the event and the
//! entry.

use std::time::Duration;

use crate::ConfigError;

use super::{
    DEFAULT_HOOK_TIMEOUT_MS, HookCommandFile, HookEntryFile, HooksFile, MAX_HOOK_ARG_BYTES,
    MAX_HOOK_ARGV, MAX_HOOK_COMMAND_BYTES, MAX_HOOK_TIMEOUT_MS, MAX_HOOKS_PER_EVENT,
};

/// The lifecycle points a hook may be attached to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    /// Before a tool call. The hook's exit code is a decision: 2 blocks.
    PreToolUse,
    /// After a tool call. The hook observes; it cannot change the outcome.
    PostToolUse,
    /// When the agent finishes a turn. The hook observes.
    Stop,
    /// Once, when an interactive session or a job root starts. The hook
    /// observes; its stdout is offered to the model as context.
    SessionStart,
    /// When the session or job root ends, normal or cancelled. The hook
    /// observes; it decides nothing.
    SessionEnd,
    /// When a call would ask the operator. Exit 2 denies the call; exit 0
    /// falls through to the normal ask. It can only narrow.
    PermissionRequest,
}

impl HookEvent {
    /// The name the event carries in the hook's own input, and in a refusal.
    /// It is the spelling the config table uses, so the three names a user
    /// writes, reads in a diagnostic, and matches on in a script are one set.
    pub const fn as_str(self) -> &'static str {
        match self {
            HookEvent::PreToolUse => "pre_tool_use",
            HookEvent::PostToolUse => "post_tool_use",
            HookEvent::Stop => "stop",
            HookEvent::SessionStart => "session_start",
            HookEvent::SessionEnd => "session_end",
            HookEvent::PermissionRequest => "permission_request",
        }
    }

    /// The name this event's input carries, in the `hook_event_name` field.
    /// The config spelling and the wire spelling differ in case only, and the
    /// wire one is the capitalized form a hook script reads.
    pub const fn wire_name(self) -> &'static str {
        match self {
            HookEvent::PreToolUse => "PreToolUse",
            HookEvent::PostToolUse => "PostToolUse",
            HookEvent::Stop => "Stop",
            HookEvent::SessionStart => "SessionStart",
            HookEvent::SessionEnd => "SessionEnd",
            HookEvent::PermissionRequest => "PermissionRequest",
        }
    }

    /// Every event, in the order the table declares them.
    pub const ALL: [HookEvent; 6] = [
        HookEvent::PreToolUse,
        HookEvent::PostToolUse,
        HookEvent::Stop,
        HookEvent::SessionStart,
        HookEvent::SessionEnd,
        HookEvent::PermissionRequest,
    ];
}

/// One hook, validated: the matcher it watches, the command to run, and the
/// timeout it runs under.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedHook {
    matcher: Option<String>,
    command: ResolvedHookCommand,
    timeout: Duration,
}

impl ResolvedHook {
    /// Whether this hook watches `tool`: an absent matcher and the wildcard
    /// `"*"` watch every tool; anything else is an exact name.
    pub fn watches(&self, tool: &str) -> bool {
        match self.matcher.as_deref() {
            None | Some("*") => true,
            Some(name) => name == tool,
        }
    }

    /// The command, as the runner hands it to the shell.
    pub fn command(&self) -> &ResolvedHookCommand {
        &self.command
    }

    /// The timeout this hook runs under.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }
}

/// The command to run. Both shapes become one `/bin/sh -c` command line, so
/// the runner has one spawn path and the bounds are the same either way.
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedHookCommand {
    /// Handed to `/bin/sh -c` verbatim.
    Shell(String),
    /// Quoted into a command line: every element is one word, and the shell
    /// interprets nothing inside one.
    Argv(Vec<String>),
}

impl ResolvedHookCommand {
    /// The command line this shape becomes. The argv form is quoted so a word
    /// carrying a space, a quote, or a `$` stays one word and expands to
    /// nothing.
    pub fn command_line(&self) -> String {
        match self {
            ResolvedHookCommand::Shell(command) => command.clone(),
            ResolvedHookCommand::Argv(words) => words
                .iter()
                .map(|word| quote(word))
                .collect::<Vec<_>>()
                .join(" "),
        }
    }
}

/// Single-quotes one word for `/bin/sh`, closing and reopening around any
/// quote it holds. This is the one escape that leaves every other byte alone.
fn quote(word: &str) -> String {
    let mut quoted = String::with_capacity(word.len() + 2);
    quoted.push('\'');
    for (index, part) in word.split('\'').enumerate() {
        if index > 0 {
            quoted.push_str("'\\''");
        }
        quoted.push_str(part);
    }
    quoted.push('\'');
    quoted
}

/// The validated hooks a surface runs, one list per event.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResolvedHooks {
    pre_tool_use: Vec<ResolvedHook>,
    post_tool_use: Vec<ResolvedHook>,
    stop: Vec<ResolvedHook>,
    session_start: Vec<ResolvedHook>,
    session_end: Vec<ResolvedHook>,
    permission_request: Vec<ResolvedHook>,
}

impl ResolvedHooks {
    /// Whether any event holds a hook. A surface with none composes no engine
    /// at all, so an unconfigured user pays nothing.
    pub fn is_empty(&self) -> bool {
        self.pre_tool_use.is_empty()
            && self.post_tool_use.is_empty()
            && self.stop.is_empty()
            && self.session_start.is_empty()
            && self.session_end.is_empty()
            && self.permission_request.is_empty()
    }

    /// The hooks attached to `event`, in the order the table declared them.
    pub fn for_event(&self, event: HookEvent) -> &[ResolvedHook] {
        match event {
            HookEvent::PreToolUse => &self.pre_tool_use,
            HookEvent::PostToolUse => &self.post_tool_use,
            HookEvent::Stop => &self.stop,
            HookEvent::SessionStart => &self.session_start,
            HookEvent::SessionEnd => &self.session_end,
            HookEvent::PermissionRequest => &self.permission_request,
        }
    }
}

/// Validates the raw table. Every bound is applied here, so the engine runs a
/// command it has already checked.
pub(crate) fn resolve(file: &HooksFile) -> Result<ResolvedHooks, ConfigError> {
    Ok(ResolvedHooks {
        pre_tool_use: resolve_event(HookEvent::PreToolUse, &file.pre_tool_use)?,
        post_tool_use: resolve_event(HookEvent::PostToolUse, &file.post_tool_use)?,
        stop: resolve_event(HookEvent::Stop, &file.stop)?,
        session_start: resolve_event(HookEvent::SessionStart, &file.session_start)?,
        session_end: resolve_event(HookEvent::SessionEnd, &file.session_end)?,
        permission_request: resolve_event(HookEvent::PermissionRequest, &file.permission_request)?,
    })
}

fn resolve_event(
    event: HookEvent,
    entries: &[HookEntryFile],
) -> Result<Vec<ResolvedHook>, ConfigError> {
    if entries.len() > MAX_HOOKS_PER_EVENT {
        return Err(ConfigError::TooManyHooks {
            event: event.as_str(),
            value: entries.len(),
            max: MAX_HOOKS_PER_EVENT,
        });
    }
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| resolve_entry(event, index + 1, entry))
        .collect()
}

fn resolve_entry(
    event: HookEvent,
    index: usize,
    entry: &HookEntryFile,
) -> Result<ResolvedHook, ConfigError> {
    let invalid = |reason: String| ConfigError::InvalidHook {
        event: event.as_str(),
        index,
        reason,
    };
    let command = match &entry.command {
        HookCommandFile::Shell(command) => {
            if command.trim().is_empty() {
                return Err(invalid("the command must not be empty".into()));
            }
            if command.len() > MAX_HOOK_COMMAND_BYTES {
                return Err(invalid(format!(
                    "the command is {} bytes; the ceiling is {MAX_HOOK_COMMAND_BYTES}",
                    command.len()
                )));
            }
            if command.contains('\0') {
                return Err(invalid("the command must not contain a NUL byte".into()));
            }
            ResolvedHookCommand::Shell(command.clone())
        }
        HookCommandFile::Argv(words) => {
            if words.is_empty() {
                return Err(invalid("the argv must name at least one word".into()));
            }
            if words.len() > MAX_HOOK_ARGV {
                return Err(invalid(format!(
                    "the argv has {} words; the ceiling is {MAX_HOOK_ARGV}",
                    words.len()
                )));
            }
            for word in words {
                if word.contains('\0') {
                    return Err(invalid("an argv word must not contain a NUL byte".into()));
                }
                if word.len() > MAX_HOOK_ARG_BYTES {
                    return Err(invalid(format!(
                        "an argv word is {} bytes; the ceiling is {MAX_HOOK_ARG_BYTES}",
                        word.len()
                    )));
                }
            }
            // The quoted command line is what the shell ceiling measures: a
            // list of maximal words quotes to more than the sum of its parts.
            let resolved = ResolvedHookCommand::Argv(words.clone());
            if resolved.command_line().len() > MAX_HOOK_COMMAND_BYTES {
                return Err(invalid(format!(
                    "the quoted argv is over the {MAX_HOOK_COMMAND_BYTES}-byte command ceiling"
                )));
            }
            resolved
        }
    };
    let timeout = match entry.timeout_ms {
        None => Duration::from_millis(DEFAULT_HOOK_TIMEOUT_MS),
        Some(0) => {
            return Err(invalid(
                "the timeout must be at least one millisecond".into(),
            ));
        }
        Some(millis) if millis > MAX_HOOK_TIMEOUT_MS => {
            return Err(invalid(format!(
                "the timeout is {millis} ms; the ceiling is {MAX_HOOK_TIMEOUT_MS} ms"
            )));
        }
        Some(millis) => Duration::from_millis(millis),
    };
    if let Some(matcher) = &entry.matcher
        && matcher.trim().is_empty()
    {
        return Err(invalid(
            "the matcher must name a tool or be \"*\", not be blank".into(),
        ));
    }
    Ok(ResolvedHook {
        matcher: entry.matcher.clone(),
        command,
        timeout,
    })
}
