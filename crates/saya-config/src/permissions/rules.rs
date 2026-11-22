//! Command-prefix permission rules: the one matcher every surface reads.
//!
//! A rule is a command *prefix*, matched token by token: `cargo test` matches
//! `cargo test -p x` and never `cargo testx`. The matcher is deliberately
//! conservative on the allow side and unconditional on the deny side:
//!
//! - A command is split into segments on `&&`, `||`, `;`, `|`, `|&`, a
//!   single `&`, a newline, and a carriage return. Every segment must match
//!   an allow rule before a call is auto-allowed, and an empty segment (a
//!   trailing separator, a backgrounded command) matches nothing.
//! - A command that contains command substitution (`$(`, backticks), process
//!   substitution (`<(`/`>(`), or a heredoc (`<<`) is never auto-allowed: the
//!   prefix the operator read is not the whole command that runs. A command
//!   with unbalanced quotes is equally unreadable and is never auto-allowed.
//! - Output redirection is not part of the prefix a rule states, except a
//!   redirect to `/dev/null` and an fd duplication (`2>&1`, `>&2`). Input
//!   redirection from a file stays covered.
//! - A deny rule is evaluated first and applies in every mode. It is the one
//!   statement the rules cannot widen, and it reads the program a command
//!   really runs: a path by its basename (`/bin/rm`, `./rm`), through a
//!   `command`/`builtin`/`exec` prefix, through an `env` with any of its
//!   options, and into a shell's `-c` script, bounded in depth. A command it
//!   cannot read, mentioning a denied program, is refused rather than allowed.
//! - A leading `cd <dir> &&` (a directory inside the workspace, or a relative
//!   one with no `..`, `$`, or `~`) is stripped for matching only. On the deny
//!   side, and only there, the shapes a program can hide behind are stripped
//!   too: leading `NAME=value` assignments, wrappers, and the openers of a
//!   subshell or a command substitution (`(`, `$(`, a backtick) on the program
//!   token itself. On the allow side an assignment or `env` makes the segment
//!   match nothing, because it can change what the program does.
//!
//! The rules are trusted-layer configuration. A project `.saya/config.toml`
//! cannot state them, and a violation of the bounds below is a diagnostic —
//! never a panic and never a silently widened rule.

use std::{fmt, path::Path};

mod resolve;
mod scan;

use resolve::mentions_program;
use scan::{scan, strip_leading_cd, tokens};

/// At most this many rules per list (`allow`, `deny`).
pub const MAX_COMMAND_RULES: usize = 64;

/// At most this many bytes in one rule.
pub const MAX_COMMAND_RULE_BYTES: usize = 256;

/// A parsed command prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRule {
    text: String,
    tokens: Vec<String>,
}

impl CommandRule {
    /// Parses one stated rule, refusing the shapes that cannot be matched
    /// exactly: empty, over the byte ceiling, or carrying a control character.
    pub fn parse(raw: &str) -> Result<Self, RuleError> {
        if raw.trim().is_empty() {
            return Err(RuleError::Empty);
        }
        if raw.len() > MAX_COMMAND_RULE_BYTES {
            return Err(RuleError::TooLong {
                bytes: raw.len(),
                max: MAX_COMMAND_RULE_BYTES,
            });
        }
        if raw.chars().any(char::is_control) {
            return Err(RuleError::ControlCharacter);
        }
        let tokens = tokens(raw);
        if tokens.is_empty() {
            return Err(RuleError::Empty);
        }
        Ok(Self {
            text: raw.to_owned(),
            tokens,
        })
    }

    /// The rule exactly as the operator stated it, for a refusal that names
    /// what matched.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The program a rule names — its first token — for the fail-closed reading
    /// of a command whose script the deny side could not read.
    fn program(&self) -> &str {
        self.tokens.first().map(String::as_str).unwrap_or_default()
    }

    fn matches(&self, segment: &[String]) -> bool {
        segment.len() >= self.tokens.len()
            && self
                .tokens
                .iter()
                .zip(segment)
                .all(|(rule, token)| rule == token)
    }

    /// Deny matching, which additionally names a program by the basename of the
    /// path it arrives as: `/bin/rm -rf x` and `./rm` are denied by `rm`.
    /// `\rm` stays a different prefix — the rule is a prefix, not a sandbox.
    fn matches_deny(&self, segment: &[String]) -> bool {
        let Some((first, rest)) = segment.split_first() else {
            return false;
        };
        let Some((rule_first, rule_rest)) = self.tokens.split_first() else {
            return false;
        };
        let first_matches = rule_first == first || names_by_path(first, rule_first);
        first_matches
            && rest.len() >= rule_rest.len()
            && rule_rest
                .iter()
                .zip(rest)
                .all(|(rule, token)| rule == token)
    }
}

/// Why a rule could not be parsed. Rendered into the config diagnostic; never
/// a panic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleError {
    Empty,
    TooLong { bytes: usize, max: usize },
    ControlCharacter,
}

impl fmt::Display for RuleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(formatter, "the rule is empty"),
            Self::TooLong { bytes, max } => {
                write!(formatter, "the rule is {bytes} bytes; the ceiling is {max}")
            }
            Self::ControlCharacter => write!(formatter, "the rule contains a control character"),
        }
    }
}

/// The resolved rules. `diagnostics` carries every stated rule that was
/// refused, so a typo is reported rather than silently inert.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandRules {
    pub allow: Vec<CommandRule>,
    pub deny: Vec<CommandRule>,
    pub diagnostics: Vec<String>,
}

/// What one command earns under the rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandDecision<'a> {
    /// A deny rule matched; the call is refused in every mode.
    Deny(&'a CommandRule),
    /// Every segment matched an allow rule, and the command carries no
    /// substitution or heredoc.
    Allow,
    /// Nothing matched; today's lane decision applies unchanged.
    Unmatched,
}

/// The one matcher. `workspace` is the shell's workspace root, when one is
/// known; it bounds the leading `cd` the allow side may strip.
pub fn decide<'a>(
    rules: &'a CommandRules,
    command: &str,
    workspace: Option<&Path>,
) -> CommandDecision<'a> {
    let scan = scan(command);
    let mut segments = scan.segments;
    strip_leading_cd(&mut segments, workspace);
    // The two sides read the segment differently. Deny resolves the program a
    // command really runs (wrappers, options, assignments, a shell's script);
    // allow reads the tokens as written, because an assignment or a wrapper can
    // change what the program does, so it is not covered by the prefix the
    // operator stated.
    let raw: Vec<Vec<String>> = segments.iter().map(|segment| tokens(segment)).collect();
    let deny = resolve::read(&segments);
    // Deny first, and regardless of substitution or readability: the refusal
    // does not depend on the allow side being parseable.
    for rule in &rules.deny {
        if deny
            .segments
            .iter()
            .any(|segment| rule.matches_deny(segment))
        {
            return CommandDecision::Deny(rule);
        }
    }
    // Fail closed: where a script was past the recursion bound, or could not be
    // read, a command that mentions a denied program anywhere is refused —
    // never allowed because the deny side ran out of ways to look.
    if deny.undecided {
        for rule in &rules.deny {
            if mentions_program(command, rule.program()) {
                return CommandDecision::Deny(rule);
            }
        }
    }
    if scan.substitution || scan.unbalanced || scan.unsafe_redirect {
        return CommandDecision::Unmatched;
    }
    // An empty segment (a trailing separator, a backgrounded command) matches
    // no rule, so it is not covered. Nothing is filtered: dropping it would
    // silently widen the command.
    if raw
        .iter()
        .all(|segment| rules.allow.iter().any(|rule| rule.matches(segment)))
    {
        return CommandDecision::Allow;
    }
    CommandDecision::Unmatched
}

/// Whether `program` names `name` by the basename of the path it is written
/// as: what a path runs is its last component, so `/bin/rm`, `./rm`, and
/// `bin/../rm` all name `rm`. A bare `rmx` names neither.
fn names_by_path(program: &str, name: &str) -> bool {
    is_path(program)
        && Path::new(program)
            .file_name()
            .and_then(|file| file.to_str())
            == Some(name)
}

/// Whether `program` is written as a path. A leading `/` is absolute in the
/// shell command language on every host, even where the host's path syntax does
/// not call it so: the deny side matches `/bin/rm` by its basename wherever the
/// rules are read.
fn is_path(program: &str) -> bool {
    Path::new(program).is_absolute() || program.contains('/')
}

#[cfg(test)]
#[path = "rules_deny_tests.rs"]
mod rules_deny_tests;

#[cfg(test)]
#[path = "rules_tests.rs"]
mod rules_tests;
