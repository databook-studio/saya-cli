//! The job shell's key: an exact command prefix, read the way the
//! `[permissions] allow` matcher reads a command.
//!
//! `shell` is the job engine's own command tool, and its call is one whole
//! shell string rather than a program plus argv, so the narrowest honest grant
//! it can name is the command's leading words: `shell:<w1> <w2>`. It is
//! suggested only when the command is **one plain segment** — when the words
//! recorded are the words that run — and never when the string carries
//! anything that would make that prefix claim untrue.
//!
//! The string is not re-read here. The one matcher for command strings is the
//! `[permissions] allow` prefix matcher ([`saya_config::decide`]), which splits
//! a command at the separators a shell honours and records the substitutions,
//! heredocs, unsafe output redirects and unbalanced quotes that make a stated
//! prefix untrue. [`is_one_plain_segment`] asks it whether the string reads as
//! a single segment stating exactly what it contains, by naming the whole
//! command as the one allowed prefix. Two refusals a session grant needs go
//! beyond the matcher's own: *any* output redirection refuses (the matcher
//! permits `/dev/null` and fd duplication), and a separator refuses even where
//! it is quoted (the matcher reads it as data, but the words the operator sees
//! in the token would then no longer be the words in front of it).

use saya_config::{CommandDecision, CommandRule, CommandRules, decide};
use serde_json::Value;

use crate::approval_facts::HostFacts;

/// The `cd` word: a leading `cd` states a directory the token does not name,
/// so everything after it is the command and the token would name the
/// directory change instead.
const LEADING_CD: &str = "cd";

/// `shell:<w1> <w2>` for a `shell` call: the command's first two words, or the
/// one word a one-word command has. Suggested only when the host route
/// composed — the same gate `command:` has, and for the same reason: a token
/// for a route the composition cannot carry would record a grant that silences
/// the ask and refuses at execution. A command that is missing, not a string,
/// empty, or not one plain segment yields `None` — never a guessed prefix.
pub(super) fn shell_token(arguments: &Value, host: Option<&HostFacts>) -> Option<String> {
    host?;
    shell_token_for_host_route(arguments)
}

/// [`shell_token`] for a caller that knows the host route composed without
/// holding the route's facts: a shell policy decides the lane itself, so the
/// one thing the gate says — this host may run the command — is the caller's
/// own answer. The key is the same one; a caller with no host route names
/// none.
pub(crate) fn shell_token_for_host_route(arguments: &Value) -> Option<String> {
    let command = arguments.get("command")?.as_str()?;
    Some(format!("shell:{}", normalised_prefix(command)?))
}

/// The **legacy** `command:<program>` key this shell call answers to, for one
/// release: the token the old `run_command` spelling recorded for a program,
/// read by the tool that replaced it.
///
/// That grant meant "this program with any argv, and no shell", so the mapping
/// is the command's own first word — the program — and only for a command that
/// is **one plain segment**: no separator, redirection, grouping, substitution,
/// leading `cd`, or leading `NAME=value`, because those are the strings whose
/// words are not the words that run. A compound command names no program token
/// at all, so a `command:` grant can never admit one.
///
/// `None` where the first word is not a bare program name — a path, a
/// traversal, or anything else the `command:` grammar would not accept — so the
/// mapping can never widen the grant it reads.
pub(crate) fn legacy_program_token(arguments: &Value) -> Option<String> {
    let command = arguments.get("command")?.as_str()?;
    let program = prefix_words(command)?.into_iter().next()?;
    saya_types::is_bare_name(program).then(|| format!("command:{program}"))
}

/// The one spelling of a `shell:` payload, for the two writers that state it:
/// the suggester, spelling a call's command, and the grammar
/// (`commands::run::scopes::parse`), spelling a typed `/allow`. Both ask this
/// function, so `/allow shell:cargo   test` and a prompt's [s] on `cargo test`
/// record one identical token — and a payload the suggester would never
/// produce is `None` here, refused there.
///
/// The words are the command's first two (or the one a one-word command has),
/// joined by single spaces exactly as the token spells them.
pub(crate) fn normalised_prefix(payload: &str) -> Option<String> {
    Some(prefix_words(payload)?.join(" "))
}

/// The command's leading words, when the command is one plain segment: at most
/// two, both shaped like the words a token may state.
fn prefix_words(command: &str) -> Option<Vec<&str>> {
    if carries_structure(command) || !is_one_plain_segment(command) {
        return None;
    }
    let mut words = command.split_whitespace();
    let first = words.next()?;
    // A leading `NAME=value` sets an environment this token does not name.
    if first == LEADING_CD || first.contains('=') {
        return None;
    }
    let mut prefix = vec![first];
    if let Some(second) = words.next() {
        prefix.push(second);
    }
    prefix
        .iter()
        .all(|word| is_plain_word(word))
        .then_some(prefix)
}

/// The strings a key never carries, whatever the matcher's segmentation says:
/// every compound separator, every redirection, a command substitution's
/// opening, and the backtick. `|` covers `||` and `|&`; `&` covers `&&`; `<`
/// covers `<<` and `<(`; `>` covers `>>`, `&>` and `>(`.
fn carries_structure(command: &str) -> bool {
    command.contains(['&', '|', ';', '<', '>', '`', '\n', '\r']) || command.contains("$(")
}

/// Whether the shared matcher reads the command as one plain segment stating
/// exactly what it contains. Stating the whole command as the one allowed
/// prefix is what makes every other reading fail: a compound command splits
/// into segments that prefix cannot match (an empty one included), and a
/// substitution, heredoc, unsafe output redirect, or unbalanced quote makes
/// the matcher refuse before any prefix is compared. A stated prefix is
/// bounded — 256 bytes, no control character — so an over-long or
/// control-bearing command reads as no segment at all and keeps asking: the
/// grammar could not state the key, so there is none to offer.
fn is_one_plain_segment(command: &str) -> bool {
    let Ok(whole) = CommandRule::parse(command) else {
        return false;
    };
    let rules = CommandRules {
        allow: vec![whole],
        deny: Vec::new(),
        diagnostics: Vec::new(),
        ..Default::default()
    };
    decide(&rules, command, None) == CommandDecision::Allow
}

/// The shape a word a key states must keep: letters, digits, and the
/// punctuation a program name, a flag, or a path is made of — never a quoting,
/// expansion, glob, or grouping character, which would make the word in the
/// token differ from the word the shell runs.
fn is_plain_word(word: &str) -> bool {
    !word.is_empty()
        && word.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(
                    character,
                    '-' | '_' | '.' | '/' | ':' | '=' | '+' | '@' | ',' | '%' | '^'
                )
        })
}
