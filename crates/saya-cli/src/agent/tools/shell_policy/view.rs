//! The shell read witness: a successful single-file view records the file's
//! current digest, so a later `edit` with no `expected_digest` is admitted
//! exactly as it would be after `read`.
//!
//! Only a command whose whole meaning is "show me this one file" counts:
//! `cat <f>`, `head [-n N] <f>`, `tail [-n N] <f>`, `sed -n '<a>,<b>p' <f>`
//! and `nl <f>`. Anything compound, redirected or piped records nothing, and
//! so does any option this list does not name — the witness must never stand
//! for a command that did more than view.

use std::sync::Arc;

use saya_harness::workspace::Workspace;

use crate::agent::tools::workspace::{ReadWitnesses, WORKSPACE_READ_MAX_BYTES};

/// The witness store and the workspace root its shell views belong to. Both
/// are the run's own: a view recorded here is visible to the same run's
/// `edit` tool and to no other run.
pub(super) struct ShellReadWitness {
    pub(super) witnesses: Arc<ReadWitnesses>,
    pub(super) workspace: Arc<Workspace>,
}

/// The one file `command` views, or `None` when the command is not exactly
/// one of the single-file views. The returned path is the command's own
/// spelling; containment is checked when it is recorded.
pub(super) fn view_path(command: &str) -> Option<String> {
    if has_shell_syntax(command) {
        return None;
    }
    let tokens = crate::interactive::session::run::tokenize_tail(command).ok()?;
    let (program, arguments) = tokens.split_first()?;
    let file = match program.as_str() {
        "cat" | "nl" => single_file(arguments)?,
        "head" | "tail" => counted_file(arguments)?,
        "sed" => sed_file(arguments)?,
        _ => return None,
    };
    (!file.is_empty() && !file.starts_with('-')).then(|| file.to_owned())
}

/// Characters whose presence means the shell would interpret the command
/// rather than pass every word to one program. Quotes and backslashes are not
/// here: the tokenizer consumes them, and a quoted argument is still one
/// word.
fn has_shell_syntax(command: &str) -> bool {
    command.chars().any(|character| {
        matches!(
            character,
            '|' | '&'
                | ';'
                | '<'
                | '>'
                | '('
                | ')'
                | '{'
                | '}'
                | '`'
                | '$'
                | '\n'
                | '\r'
                | '*'
                | '?'
                | '['
                | ']'
                | '~'
                | '#'
                | '!'
        )
    })
}

/// `cat <f>` or `nl <f>`: exactly one argument and no options.
fn single_file(arguments: &[String]) -> Option<&str> {
    match arguments {
        [file] if !file.starts_with('-') => Some(file),
        _ => None,
    }
}

/// `head <f>`, `head -n N <f>`, and the same for `tail`. Any other option —
/// `-f` included — is not a one-shot view and records nothing.
fn counted_file(arguments: &[String]) -> Option<&str> {
    match arguments {
        [file] if !file.starts_with('-') => Some(file),
        [flag, count, file]
            if flag == "-n"
                && !count.is_empty()
                && count.chars().all(|character| character.is_ascii_digit())
                && !file.starts_with('-') =>
        {
            Some(file)
        }
        _ => None,
    }
}

/// `sed -n '<a>,<b>p' <f>`: a numeric address range and one file.
fn sed_file(arguments: &[String]) -> Option<&str> {
    match arguments {
        [flag, script, file]
            if flag == "-n"
                && !file.starts_with('-')
                && script.strip_suffix('p').is_some_and(numeric_range) =>
        {
            Some(file)
        }
        _ => None,
    }
}

fn numeric_range(range: &str) -> bool {
    let Some((start, end)) = range.split_once(',') else {
        return false;
    };
    [start, end].iter().all(|number| {
        !number.is_empty() && number.chars().all(|character| character.is_ascii_digit())
    })
}

/// Records `command`'s viewed file when it is one, the file is inside the
/// workspace, and it fits the read bound. The digest is the file's state at
/// this moment — right after the shell call — so a later edit resolves against
/// exactly what the model saw.
pub(super) fn record(target: &ShellReadWitness, command: &str) {
    let Some(path) = view_path(command) else {
        return;
    };
    let Ok(relative) = target.workspace.relative_argument(&path) else {
        return;
    };
    let Ok(file) = target
        .workspace
        .read(relative.as_ref(), WORKSPACE_READ_MAX_BYTES)
    else {
        return;
    };
    if file.size > WORKSPACE_READ_MAX_BYTES {
        return;
    }
    target
        .witnesses
        .remember_view(relative.as_ref(), &target.workspace, file.digest, file.size);
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
