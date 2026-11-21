//! The knowledge store's verbs: `/contracts list|show|queue|confirm|reject|confirm-all|export|import`,
//! and the four deprecated spellings that name one of them.
//!
//! The verb word is the first token, exactly as the headless `saya contracts`
//! subcommand word is. A lone token that is not a verb stays a Show, so
//! `/contracts catalog.schema.object` (today's form) keeps working. The
//! deprecated spellings reach the **same** parser arms — `/queue` is
//! `parse_queue("queue", …)`, `/contracts queue` is `parse_queue("contracts
//! queue", …)` — so one grammar answers both, and only the usage line differs
//! (it names the spelling the user typed).

use super::decide::parse_decide;
use super::portable::parse_portable;
use super::usage_contracts;
use crate::cli::{ContractsCommand, ReviewDecisionArg};
use crate::slash::SlashParseError;

/// The verbs a `/contracts` first token may name. Anything else is a table.
const VERBS: [&str; 8] = [
    "list",
    "show",
    "queue",
    "confirm",
    "reject",
    "confirm-all",
    "export",
    "import",
];

/// Parses the tail after `/contracts` (or `/contract`): the first token selects
/// the verb, the rest is that verb's argument grammar.
pub(super) fn parse_contracts(arg: &str) -> Result<ContractsCommand, SlashParseError> {
    let tokens: Vec<&str> = arg.split_whitespace().collect();
    match tokens.first().copied() {
        None => Ok(ContractsCommand::List { profile: None }),
        // `import-dbt` is the third portable operation: it is not one of the
        // eight verbs the row advertises, and it keeps working as today.
        Some(word @ "import-dbt") => parse_portable(word, &tokens[1..].join(" ")),
        Some(word) if VERBS.contains(&word) => {
            parse_verb(&format!("contracts {word}"), word, &tokens[1..])
        }
        Some(table) if tokens.len() == 1 => Ok(ContractsCommand::Show {
            table: table.to_string(),
            profile: None,
        }),
        Some(_) => Err(SlashParseError(usage_contracts())),
    }
}

/// Parses one of the four deprecated spellings through the same verb grammar
/// the canonical form uses. `spelling` is the name the user typed, so the usage
/// line and any refusal name it.
pub(super) fn parse_alias(spelling: &str, arg: &str) -> Result<ContractsCommand, SlashParseError> {
    let verb = alias_verb(spelling);
    let tokens: Vec<&str> = arg.split_whitespace().collect();
    parse_verb(spelling, verb, &tokens)
}

/// The verb a deprecated spelling names.
fn alias_verb(spelling: &str) -> &'static str {
    match spelling {
        "approve-all" => "confirm-all",
        "queue" => "queue",
        "confirm" => "confirm",
        _ => "reject",
    }
}

/// Parses one verb's argument grammar, shared by the canonical and the alias
/// spelling. `spelling` is the form as typed — `contracts queue` or `queue` —
/// so a usage error names the form the user actually typed.
fn parse_verb(
    spelling: &str,
    verb: &str,
    rest: &[&str],
) -> Result<ContractsCommand, SlashParseError> {
    match verb {
        "list" => {
            if rest.is_empty() {
                Ok(ContractsCommand::List { profile: None })
            } else {
                Err(SlashParseError(usage_contracts()))
            }
        }
        "show" => match rest {
            [table] => Ok(ContractsCommand::Show {
                table: (*table).to_string(),
                profile: None,
            }),
            _ => Err(SlashParseError(format!("/{spelling} <table>"))),
        },
        "queue" => parse_queue(spelling, rest),
        "confirm" => parse_decide(spelling, rest, ReviewDecisionArg::Confirm),
        "reject" => parse_decide(spelling, rest, ReviewDecisionArg::Reject),
        "confirm-all" => parse_batch(spelling, rest),
        // The portable operations take a path and fixed flags; the tail is
        // re-joined because their parser reads it as one argument.
        word => parse_portable(word, &rest.join(" ")),
    }
}

/// `/contracts queue [limit]` and its deprecated `queue` spelling: the optional
/// positional number overrides the default limit.
fn parse_queue(spelling: &str, rest: &[&str]) -> Result<ContractsCommand, SlashParseError> {
    let usage = || SlashParseError(format!("/{spelling} [limit]"));
    let limit = match rest {
        [] => None,
        [limit] => Some(limit.parse::<usize>().map_err(|_| usage())?),
        _ => return Err(usage()),
    };
    Ok(ContractsCommand::Queue {
        profile: None,
        limit,
    })
}

/// `/contracts confirm-all [--yes] [limit]` and its deprecated `approve-all`
/// spelling. Without `--yes` the queue is previewed and nothing is approved —
/// deny by default, the same posture `--non-interactive` gives approvals.
fn parse_batch(spelling: &str, rest: &[&str]) -> Result<ContractsCommand, SlashParseError> {
    let usage = || SlashParseError(format!("/{spelling} [--yes] [limit]"));
    let mut yes = false;
    let mut limit = None;
    for token in rest {
        if *token == "--yes" {
            if yes {
                return Err(usage());
            }
            yes = true;
        } else if let Ok(parsed) = token.parse::<usize>() {
            if limit.is_some() {
                return Err(usage());
            }
            limit = Some(parsed);
        } else {
            return Err(usage());
        }
    }
    Ok(ContractsCommand::ApproveAll {
        profile: None,
        limit,
        yes,
    })
}
