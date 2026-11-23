//! The deny side's reading of a command: the program it really runs.
//!
//! The deny side answers one question — does this command run the program a
//! deny rule names? — and it must not be talked out of the answer by the shapes
//! a program can arrive in: a `command`/`builtin`/`exec` prefix, an `env` with
//! any of its options, a leading `NAME=value`, the openers of a subshell or a
//! command substitution, a path whose basename is the program, a launcher that
//! runs another program (`timeout`, `nice`, `nohup`, `stdbuf`, `time`, `xargs`,
//! `find ... -exec`), or a script (`sh -c`, `eval`). A script is read with this
//! same reading, at most [`MAX_SCRIPT_DEPTH`] deep; past that, where a script
//! cannot be read, and where an interpreter's inline code is past reading, the
//! reading reports itself undecided and the matcher falls back to refusing a
//! command that mentions a denied program anywhere.
//!
//! None of this is the allow side's: what an approval grants is the prefix the
//! operator stated, and every one of these shapes makes a segment match no
//! allow rule.

use std::path::Path;

use super::launch::{self, Nested};
use super::scan::{scan, tokens};

/// At most this many nested scripts are read. A script deeper than this is not
/// read, and its command is refused rather than allowed.
pub(super) const MAX_SCRIPT_DEPTH: usize = 3;

/// One command read for the deny side: every segment resolved to the program it
/// runs — a nested script's segments included — and whether some script could
/// not be read.
pub(super) struct Reading {
    pub(super) segments: Vec<Vec<String>>,
    pub(super) undecided: bool,
}

/// Reads every segment, plus every script and launcher program it runs.
pub(super) fn read(segments: &[String]) -> Reading {
    let mut reading = Reading {
        segments: Vec::new(),
        undecided: false,
    };
    let mut pending: Vec<(Vec<String>, usize)> = segments
        .iter()
        .map(|segment| (tokens(segment), 0))
        .collect();
    while let Some((raw, depth)) = pending.pop() {
        let resolved = resolve(raw);
        let Some((program, arguments)) = resolved.split_first() else {
            continue;
        };
        let nested = launch::nested(program, arguments);
        reading.segments.push(resolved);
        let scripts = match nested {
            Nested::None => continue,
            Nested::Unreadable => {
                reading.undecided = true;
                continue;
            }
            Nested::Segments(scripts) => scripts,
        };
        if depth + 1 > MAX_SCRIPT_DEPTH {
            reading.undecided = true;
            continue;
        }
        for script in scripts {
            let script = scan(&script);
            if script.unbalanced {
                reading.undecided = true;
                continue;
            }
            for segment in script.segments {
                pending.push((tokens(&segment), depth + 1));
            }
        }
    }
    reading
}

/// The tokens a segment really runs with: the wrappers, options, and
/// assignments a program can arrive behind are skipped, and the program token
/// loses the characters that open a subshell or a command substitution, so
/// `$(curl x)` and `( curl x )` name `curl` exactly as `curl x` does.
///
/// The read-only set reads a segment through this too: a segment it changes is
/// one whose written program is not the program that runs.
pub(super) fn resolve(tokens: Vec<String>) -> Vec<String> {
    let mut start = 0;
    while let Some(token) = tokens.get(start) {
        let skip = if is_env_assignment(token) || is_opener_only(token) {
            1
        } else {
            // A wrapper runs what follows it; its own options are not the
            // program, nor are the values they carry.
            launch::wrapper_skip(&tokens[start..]).unwrap_or_default()
        };
        if skip == 0 {
            break;
        }
        start += skip;
    }
    let mut rest = tokens[start..].to_vec();
    if let Some(program) = rest.first_mut() {
        *program = program.trim_start_matches(['(', '$', '`']).to_owned();
    }
    rest
}

/// Whether `command` names `program` as a whole word — the fail-closed reading
/// of a command whose script the deny side could not read. A program that
/// merely begins with the name (`curling` for `curl`) does not qualify.
pub(super) fn mentions_program(command: &str, program: &str) -> bool {
    command
        .split(|character: char| {
            !(character.is_alphanumeric() || matches!(character, '_' | '-' | '/' | '.'))
        })
        .any(|word| {
            word == program
                || Path::new(word).file_name().and_then(|name| name.to_str()) == Some(program)
        })
}

/// Whether `program` names `name` by the basename of the path it is written
/// as: what a path runs is its last component, so `/bin/rm`, `./rm`, and
/// `bin/../rm` all name `rm`. A bare `rmx` names neither.
pub(super) fn names_by_path(program: &str, name: &str) -> bool {
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

/// Whether the token is nothing but the characters that open a subshell or a
/// command substitution, which name no program of their own.
fn is_opener_only(token: &str) -> bool {
    !token.is_empty()
        && token
            .chars()
            .all(|character| matches!(character, '(' | '$' | '`'))
}

fn is_env_assignment(token: &str) -> bool {
    let Some((name, _)) = token.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_alphabetic() || first == '_')
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
}
