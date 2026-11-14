//! User-authored slash commands: markdown files in a `commands/` directory
//! become `/<name>` prompts.
//!
//! Two directories are read, in this order: the user's own
//! `<config home>/commands/`, then the workspace's `.saya/commands/`. The
//! project directory is **untrusted input** — it comes from the repository —
//! so both kinds load identically and a match is handed straight to the agent
//! as the user's message. A command file can never change configuration, the
//! approval mode, the tool set, or a permission: nothing in it is executed,
//! and an expansion is never re-parsed as a slash command.
//!
//! Bounds are explicit: [`MAX_COMMAND_FILES`] files, [`MAX_COMMAND_FILE_BYTES`]
//! bytes per file, [`MAX_EXPANDED_PROMPT_BYTES`] for the expanded prompt, and
//! [`MAX_COMMAND_NAME_CHARS`] in a name. A file that breaks one is refused
//! with a diagnostic, never truncated into something else.

use crate::slash::registry;
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// The placeholder replaced by the text typed after the command.
const ARGUMENTS_PLACEHOLDER: &str = "$ARGUMENTS";
/// At most this many command files load, across both directories.
const MAX_COMMAND_FILES: usize = 64;
/// Byte ceiling for one command file. A larger file is refused, never
/// truncated.
const MAX_COMMAND_FILE_BYTES: usize = 8 * 1024;
/// Byte ceiling for the expanded prompt sent as the user's message.
const MAX_EXPANDED_PROMPT_BYTES: usize = 32 * 1024;
/// Character ceiling for a command name.
const MAX_COMMAND_NAME_CHARS: usize = 64;
/// At most this many load diagnostics are reported, so a hostile directory
/// cannot flood a session's startup with them.
const MAX_COMMAND_NOTICES: usize = 16;

/// Where a command file was read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandSource {
    /// `<config home>/commands/` — the user's own files.
    User,
    /// `<workspace>/.saya/commands/` — the repository's files. Untrusted,
    /// and treated exactly like a user command: it can only ever expand to a
    /// user message.
    Project,
    /// Shipped with saya. It is an ordinary command body, and a user or
    /// project file with the same name replaces it.
    Builtin,
}

impl CommandSource {
    fn label(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
            Self::Builtin => "built-in",
        }
    }
}

/// One loaded command file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UserCommand {
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    /// The prompt body: front matter removed, outer ends trimmed.
    pub(crate) body: String,
    pub(crate) source: CommandSource,
    pub(crate) path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Entry {
    Ready(UserCommand),
    /// A file that claimed a name and was refused. The name stays reserved —
    /// a later file cannot take it — and typing it says why.
    Refused {
        path: PathBuf,
        reason: String,
    },
}

/// What one typed line is, as far as the user commands are concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UserLine {
    /// Not a user command: the built-in parser owns the line.
    Builtin,
    /// The prompt the agent receives. The expansion is never re-parsed as a
    /// slash command, so a command file can only ever produce a user message.
    Prompt(String),
    /// The refusal to say.
    Refused(String),
}

/// The commands loaded for this session, plus the loader's diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UserCommands {
    entries: BTreeMap<String, Entry>,
    notices: Vec<String>,
    notices_suppressed: bool,
}

impl Default for UserCommands {
    /// A session always has the shipped defaults; `load` then lets the user's
    /// and the repository's files replace them by name.
    fn default() -> Self {
        let mut commands = Self {
            entries: BTreeMap::new(),
            notices: Vec::new(),
            notices_suppressed: false,
        };
        seed_builtins(&mut commands);
        commands
    }
}

impl UserCommands {
    /// The load diagnostics, in load order: every refusal and every ignored
    /// front-matter key. Bounded by `MAX_COMMAND_NOTICES`.
    pub(crate) fn notices(&self) -> &[String] {
        &self.notices
    }

    /// The loaded commands, name-ordered. A refused file is not a command and
    /// is not listed.
    pub(crate) fn ready(&self) -> impl Iterator<Item = &UserCommand> {
        self.entries.values().filter_map(|entry| match entry {
            Entry::Ready(command) => Some(command),
            Entry::Refused { .. } => None,
        })
    }

    /// The loaded command with this name, if one loaded.
    pub(crate) fn get(&self, name: &str) -> Option<&UserCommand> {
        match self.entries.get(name) {
            Some(Entry::Ready(command)) => Some(command),
            _ => None,
        }
    }

    /// What a typed line is: a prompt for the agent, a refusal, or a line the
    /// built-in parser owns. The name is everything up to the first space;
    /// the arguments keep their interior whitespace verbatim, the `/sql`
    /// idiom.
    pub(crate) fn route(&self, input: &str) -> UserLine {
        let Some(rest) = input.trim().strip_prefix('/') else {
            return UserLine::Builtin;
        };
        let (name, args) = match rest.split_once(char::is_whitespace) {
            Some((name, args)) => (name, args.trim()),
            None => (rest, ""),
        };
        match self.entries.get(name) {
            Some(Entry::Ready(command)) => match expand(command, args) {
                Ok(prompt) => UserLine::Prompt(prompt),
                Err(reason) => UserLine::Refused(reason),
            },
            Some(Entry::Refused { reason, .. }) => {
                UserLine::Refused(format!("/{name} was refused: {reason}"))
            }
            None => UserLine::Builtin,
        }
    }
}

include!("load.rs");
include!("parse.rs");
include!("expand.rs");
include!("help.rs");
include!("builtin.rs");

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
