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
//!   statement the rules cannot widen, and it matches an absolute program by
//!   its basename (`/bin/rm` matches deny `rm`).
//! - A leading `cd <dir> &&` (a directory inside the workspace, or a relative
//!   one with no `..`, `$`, or `~`) is stripped for matching only. Leading
//!   `NAME=value` assignments are stripped for the deny side only: on the
//!   allow side they make the segment match nothing.
//!
//! The rules are trusted-layer configuration. A project `.saya/config.toml`
//! cannot state them, and a violation of the bounds below is a diagnostic —
//! never a panic and never a silently widened rule.

use std::{
    fmt,
    path::{Component, Path},
};

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

    fn matches(&self, segment: &[String]) -> bool {
        segment.len() >= self.tokens.len()
            && self
                .tokens
                .iter()
                .zip(segment)
                .all(|(rule, token)| rule == token)
    }

    /// Deny matching, which additionally names an absolute program by its
    /// basename: `/bin/rm -rf x` is denied by `rm`. `command rm` and `\rm`
    /// stay different prefixes — the rule is a prefix, not a sandbox.
    fn matches_deny(&self, segment: &[String]) -> bool {
        let Some((first, rest)) = segment.split_first() else {
            return false;
        };
        let Some((rule_first, rule_rest)) = self.tokens.split_first() else {
            return false;
        };
        let first_matches = rule_first == first
            || (Path::new(first).is_absolute()
                && Path::new(first).file_name().and_then(|name| name.to_str())
                    == Some(rule_first.as_str()));
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
    // The two sides read the segment differently. Deny strips a leading
    // assignment so `FOO=1 rm` cannot hide behind one. Allow does not: an
    // assignment can change what the program does, so it is not covered by
    // the prefix the operator stated.
    let raw: Vec<Vec<String>> = segments.iter().map(|segment| tokens(segment)).collect();
    let deny: Vec<Vec<String>> = raw.iter().map(|tokens| strip_env(tokens)).collect();
    // Deny first, and regardless of substitution or readability: the refusal
    // does not depend on the allow side being parseable.
    for rule in &rules.deny {
        if deny.iter().any(|segment| rule.matches_deny(segment)) {
            return CommandDecision::Deny(rule);
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

/// A command split into segments, and the facts that make a prefix claim
/// untrue: a substitution, an unsafe output redirect, or unbalanced quotes.
struct Scan {
    segments: Vec<String>,
    substitution: bool,
    unsafe_redirect: bool,
    unbalanced: bool,
}

fn scan(command: &str) -> Scan {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut chars = command.chars().peekable();
    let (mut single, mut double) = (false, false);
    let mut substitution = false;
    let mut unsafe_redirect = false;
    while let Some(c) = chars.next() {
        match c {
            '\\' if !single => match chars.peek().copied() {
                // A backslash-newline joins the lines; it is not a separator.
                Some('\n') => {
                    chars.next();
                }
                Some('\r') => {
                    chars.next();
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                }
                Some(next) => {
                    current.push(c);
                    current.push(next);
                    chars.next();
                }
                None => current.push(c),
            },
            '\'' if !double => {
                single = !single;
                current.push(c);
            }
            '"' if !single => {
                double = !double;
                current.push(c);
            }
            '`' if !single => {
                substitution = true;
                current.push(c);
            }
            '$' if !single => {
                current.push(c);
                if chars.peek() == Some(&'(') {
                    substitution = true;
                    current.push(chars.next().expect("peeked"));
                }
            }
            '<' if !single && !double => {
                current.push(c);
                if matches!(chars.peek(), Some('<') | Some('(')) {
                    substitution = true;
                }
            }
            '>' if !single && !double => {
                current.push(c);
                if matches!(chars.peek(), Some('>') | Some('|')) {
                    current.push(chars.next().expect("peeked"));
                }
                unsafe_redirect |= !consume_redirect_target(&mut chars);
            }
            '&' if !single && !double => match chars.peek() {
                Some('&') => {
                    chars.next();
                    segments.push(std::mem::take(&mut current));
                }
                // `&>` is a redirect form, not a background operator.
                Some('>') => current.push(c),
                _ => segments.push(std::mem::take(&mut current)),
            },
            '|' if !single && !double => {
                if matches!(chars.peek(), Some('|') | Some('&')) {
                    chars.next();
                }
                segments.push(std::mem::take(&mut current));
            }
            ';' | '\n' | '\r' if !single && !double => {
                // CRLF is one separator, not two.
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                segments.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    segments.push(current);
    Scan {
        segments,
        substitution,
        unsafe_redirect,
        unbalanced: single || double,
    }
}

/// Consumes the word after an output redirect and reports whether the allow
/// side may cover it: `/dev/null` and an fd duplication (`>&2`) are safe,
/// every other target is not.
fn consume_redirect_target(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> bool {
    while matches!(chars.peek(), Some(' ') | Some('\t')) {
        chars.next();
    }
    if chars.peek() == Some(&'&') {
        let mut lookahead = chars.clone();
        lookahead.next();
        let digits: String = lookahead
            .by_ref()
            .take_while(char::is_ascii_digit)
            .collect();
        if digits.is_empty() {
            return false;
        }
        chars.next();
        for _ in 0..digits.len() {
            chars.next();
        }
        return true;
    }
    let mut word = String::new();
    while let Some(&next) = chars.peek() {
        if next.is_whitespace() || matches!(next, '&' | '|' | ';' | '<' | '>') {
            break;
        }
        word.push(chars.next().expect("peeked"));
    }
    word.trim_matches(|c| c == '\'' || c == '"') == "/dev/null"
}

/// Splits a segment into tokens. Whitespace separates; a matching pair of
/// surrounding quotes is removed from each token so `echo "x"` and
/// `echo x` match the same rule.
fn tokens(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|token| {
            let token = token.trim_matches(|c| c == '\'' || c == '"');
            token.to_owned()
        })
        .collect()
}

/// Strips one leading `cd <dir>` segment when it is the first of several and
/// the directory stays inside the workspace (or is relative with no `..`).
fn strip_leading_cd(segments: &mut Vec<String>, workspace: Option<&Path>) {
    if segments.len() < 2 {
        return;
    }
    let first = tokens(&segments[0]);
    if first.len() == 2 && first[0] == "cd" && cd_within(&first[1], workspace) {
        segments.remove(0);
    }
}

fn cd_within(dir: &str, workspace: Option<&Path>) -> bool {
    // `cd -`, `cd ~`, and a shell expansion are not known to stay inside the
    // workspace, so the segment is not stripped and must match a rule itself.
    if dir == "-" || dir.contains('$') || dir.contains('~') {
        return false;
    }
    let path = Path::new(dir);
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return false;
    }
    if path.is_absolute() {
        let Some(root) = workspace else {
            return false;
        };
        return path
            .canonicalize()
            .map(|path| path.starts_with(root))
            .unwrap_or(false);
    }
    true
}

/// Strips leading `NAME=value` assignments for the deny side. The first
/// non-assignment token is the program; a segment made only of assignments
/// matches nothing.
fn strip_env(tokens: &[String]) -> Vec<String> {
    let start = tokens
        .iter()
        .position(|token| !is_env_assignment(token))
        .unwrap_or(tokens.len());
    tokens[start..].to_vec()
}

fn is_env_assignment(token: &str) -> bool {
    let Some((name, _)) = token.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
#[path = "rules_tests.rs"]
mod rules_tests;
