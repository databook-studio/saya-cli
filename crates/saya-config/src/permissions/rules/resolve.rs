//! The deny side's reading of a command: the program it really runs.
//!
//! The deny side answers one question — does this command run the program a
//! deny rule names? — and it must not be talked out of the answer by the shapes
//! a program can arrive in: a `command`/`builtin`/`exec` prefix, an `env` with
//! any of its options, a leading `NAME=value`, the openers of a subshell or a
//! command substitution, a path whose basename is the program, or another
//! shell's `-c` script. A script is read with this same reading, at most
//! [`MAX_SCRIPT_DEPTH`] deep; past that, or where a script cannot be read, the
//! reading reports itself undecided and the matcher falls back to refusing a
//! command that mentions a denied program anywhere.
//!
//! None of this is the allow side's: what an approval grants is the prefix the
//! operator stated, and every one of these shapes makes a segment match no
//! allow rule.

use std::path::Path;

use super::scan::{scan, tokens};

/// At most this many nested `-c` scripts are read. A script deeper than this is
/// not read, and its command is refused rather than allowed.
pub(super) const MAX_SCRIPT_DEPTH: usize = 3;

/// One command read for the deny side: every segment resolved to the program it
/// runs — a nested script's segments included — and whether some script could
/// not be read.
pub(super) struct Reading {
    pub(super) segments: Vec<Vec<String>>,
    pub(super) undecided: bool,
}

/// Reads every segment, plus every script a shell segment runs.
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
        let script = script(program, arguments);
        reading.segments.push(resolved);
        let Some(script) = script else {
            continue;
        };
        if depth + 1 > MAX_SCRIPT_DEPTH {
            reading.undecided = true;
            continue;
        }
        let script = scan(&script);
        if script.unbalanced {
            reading.undecided = true;
            continue;
        }
        for segment in script.segments {
            pending.push((tokens(&segment), depth + 1));
        }
    }
    reading
}

/// The tokens a segment really runs with: the wrappers, options, and
/// assignments a program can arrive behind are skipped, and the program token
/// loses the characters that open a subshell or a command substitution, so
/// `$(curl x)` and `( curl x )` name `curl` exactly as `curl x` does.
fn resolve(tokens: Vec<String>) -> Vec<String> {
    let mut start = 0;
    while let Some(token) = tokens.get(start) {
        let followed = start + 1 < tokens.len();
        let skip = if is_env_assignment(token) || is_opener_only(token) {
            1
        } else if is_wrapper(token) && followed {
            // A wrapper runs what follows it; its options are not the program.
            1 + option_count(&tokens[start + 1..])
        } else {
            0
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

/// The programs that run another program named after them. `env` sets the
/// environment, `command` and `builtin` bypass a function of the same name, and
/// `exec` replaces this shell with the program — in every case what follows is
/// the program that runs.
fn is_wrapper(token: &str) -> bool {
    matches!(token, "env" | "command" | "builtin" | "exec")
}

/// The leading options of a wrapped program, with the value of an option that
/// carries one, so a value is never mistaken for the program.
fn option_count(tokens: &[String]) -> usize {
    let mut skipped = 0;
    while let Some(token) = tokens.get(skipped) {
        if token == "--" {
            return skipped + 1;
        }
        if !is_option(token) {
            break;
        }
        skipped += 1;
        if takes_a_value(token) && tokens.get(skipped).is_some() {
            skipped += 1;
        }
    }
    skipped
}

fn is_option(token: &str) -> bool {
    token.starts_with('-') && token.len() > 1
}

/// The options whose value is the next token: `env -u NAME`, `env -C DIR`,
/// `env -S STRING`, `exec -a NAME`.
fn takes_a_value(option: &str) -> bool {
    matches!(
        option,
        "-u" | "-C" | "-S" | "-a" | "--unset" | "--chdir" | "--split-string"
    )
}

/// The `-c` script a shell runs, when this segment is one: `sh -c 'x'`,
/// `bash -lc 'x'`, `zsh -c "x"`, `dash -c 'x'`. The script runs to the end of
/// the segment, so anything after the flag is part of what the shell reads.
fn script(program: &str, arguments: &[String]) -> Option<String> {
    if !matches!(program, "sh" | "bash" | "zsh" | "dash") {
        return None;
    }
    let flag = arguments
        .iter()
        .position(|argument| runs_a_script(argument))?;
    let script = arguments[flag + 1..].join(" ");
    (!script.is_empty()).then_some(script)
}

/// Whether this argument is a shell's `-c`, or a cluster holding it (`-lc`,
/// `-ec`): the next token is the script the shell runs.
fn runs_a_script(argument: &str) -> bool {
    argument.starts_with('-') && !argument.starts_with("--") && argument.contains('c')
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
