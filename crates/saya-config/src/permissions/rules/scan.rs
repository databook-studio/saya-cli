//! Reading a shell command string: segmentation, tokens, substitutions, and
//! the leading `cd` the matcher strips before it compares prefixes.
//!
//! The scanner is deliberately textual. It expands nothing; it records the
//! facts that make a stated prefix untrue (a substitution, an unsafe redirect,
//! unbalanced quotes) and splits the command at the separators a shell would
//! honour.

use std::path::{Component, Path};

/// A command split into segments, and the facts that make a prefix claim
/// untrue: a substitution, an unsafe output redirect, or unbalanced quotes.
pub(super) struct Scan {
    pub(super) segments: Vec<String>,
    pub(super) substitution: bool,
    pub(super) unsafe_redirect: bool,
    pub(super) unbalanced: bool,
}

pub(super) fn scan(command: &str) -> Scan {
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
pub(super) fn tokens(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|token| {
            let token = token.trim_matches(|c| c == '\'' || c == '"');
            token.to_owned()
        })
        .collect()
}

/// Strips one leading `cd <dir>` segment when it is the first of several and
/// the directory stays inside the workspace (or is relative with no `..`).
pub(super) fn strip_leading_cd(segments: &mut Vec<String>, workspace: Option<&Path>) {
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
    // Rooted is not the same as absolute: `/tmp` has no drive letter and so is
    // not absolute on Windows, but it is still anchored, never a relative
    // directory the workspace contains.
    let rooted = path
        .components()
        .any(|component| matches!(component, Component::Prefix(_) | Component::RootDir));
    if rooted {
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
