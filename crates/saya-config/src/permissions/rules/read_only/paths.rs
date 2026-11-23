//! The path rule: what a vetted command may name.
//!
//! A vetted command runs on the host without a question, so every name it
//! carries is what the operator has consented to read. The rule is the read
//! tool's own, applied to the command line instead of to a `read` argument:
//!
//! - every positional argument, and every value an option carries, is a path
//!   this rule reads — `grep -f FILE`, `diff A B`, `head ../x/.env`;
//! - it must be **relative**: no leading `/`, no `~`, no drive or UNC shape;
//! - it must stay inside the workspace after **lexical** normalisation: a `..`
//!   may cancel a component, never rise above the root (`src/../../x` is
//!   `../x`);
//! - it must not match the read tool's private-path set
//!   ([`sensitive_path`]) — `.env`, a key or certificate, the credential
//!   directories. That is this crate's own list; the rest of the read tool's
//!   rule (the store's screen and the shared redactor) lives in the harness,
//!   which a configuration crate cannot see, so the shell's own policy reads the
//!   names this rule vets against those two screens as well
//!   ([`super::read_only::named_paths`]);
//! - `/dev/null` is the one absolute name that is allowed, because it is the
//!   null device rather than a file.
//!
//! Anything else keeps the question. The rule errs towards asking: a word that
//! merely looks like a path is read as one, and a token's own option letters
//! are stripped so a value glued to a short option (`-f.env`) is read too.

use crate::sensitive::sensitive_path;

/// Whether every path this argument list names is one the workspace tools
/// would also read.
pub(super) fn vetted(arguments: &[String]) -> bool {
    arguments.iter().all(|argument| argument_vetted(argument))
}

/// Whether one argument names no path outside the rule. An option's own
/// letters name nothing; the value glued to it — after `=`, or after a short
/// option's letter — is read as a path.
fn argument_vetted(argument: &str) -> bool {
    spellings(argument).all(path_vetted)
}

/// Every spelling of a path one token states: the token itself, and the
/// remainders after each separator an option uses to attach a value
/// (`--file=../x`, `-f.env`, `-C/tmp`).
pub(super) fn spellings(argument: &str) -> impl Iterator<Item = &str> {
    std::iter::once(argument).chain(
        argument
            .char_indices()
            .filter(|(_, character)| matches!(character, '-' | '='))
            .map(|(index, _)| &argument[index + 1..]),
    )
}

/// Whether one spelling names no path outside the rule.
fn path_vetted(spelling: &str) -> bool {
    let spelling = spelling.trim_matches(|character| character == '\'' || character == '"');
    if spelling.is_empty() || spelling == "-" || spelling == "/dev/null" {
        return true;
    }
    // A rooted path, a home directory, a Windows separator, or a drive shape
    // is never a relative name inside the workspace.
    if spelling.starts_with('/')
        || spelling.contains('~')
        || spelling.contains('\\')
        || is_drive(spelling)
    {
        return false;
    }
    inside_root(spelling) && !sensitive_path(spelling)
}

/// Whether a relative path stays inside the root once its `.` and `..`
/// components are cancelled lexically.
fn inside_root(spelling: &str) -> bool {
    let mut depth: i32 = 0;
    for component in spelling.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => depth += 1,
        }
    }
    true
}

/// Whether the name is a Windows drive or an alternate data stream (`C:x`,
/// `C:/x`), which is rooted on that host whatever this one's syntax says.
fn is_drive(spelling: &str) -> bool {
    let mut characters = spelling.chars();
    matches!(
        (characters.next(), characters.next()),
        (Some(letter), Some(':')) if letter.is_ascii_alphabetic()
    )
}
