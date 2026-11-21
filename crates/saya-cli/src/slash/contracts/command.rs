//! The contract slash dispatcher: translates a slash command name plus its
//! argument tail into the matching `ContractsCommand`, or a payload-free usage
//! error. Each arm produces the same typed value the headless `saya contracts`
//! parser produces, so the slash and headless paths hand one value to the
//! shared `run_contracts` dispatcher — no second parsing, privacy decision, or
//! DTO mapping lives here.
//!
//! Every spelling parses to a [`ContractsRequest`]: the one operation plus, for
//! a deprecated spelling, the one-line note it states. The operation itself is
//! parsed once, so `/confirm ki-a86a3f` and `/contracts confirm ki-a86a3f`
//! cannot disagree about what they do.

mod decide;
mod portable;
mod verbs;

use super::alias::ContractsRequest;
use super::remember::parse_remember;
use crate::cli::{ContractsCommand, ForgetReasonArg};
use crate::slash::SlashParseError;

/// Payload-free usage for the merged `/contracts` command. Never echoes the
/// untrusted tail — a two-token argument is a usage error, not a Show of either
/// token. The first token selects the operation: absent → list, a verb word →
/// that verb, anything else → show.
fn usage_contracts() -> String {
    "/contracts [catalog.schema.object]".into()
}

/// Payload-free usage for `/forget`.
fn usage_forget() -> String {
    "/forget <claim-id>".into()
}

/// Translates a slash command name + its argument tail into the matching
/// `ContractsRequest` — the one operation, plus the deprecation note a
/// deprecated spelling states — or a usage error. The argument is the raw text
/// after the command word (already trimmed of the leading `/name`).
pub(crate) fn parse_contract_command(
    name: &str,
    arg: &str,
) -> Result<Option<ContractsRequest>, SlashParseError> {
    let Some(command) = parse_operation(name, arg)? else {
        return Ok(None);
    };
    Ok(Some(ContractsRequest::stating(name, command)))
}

/// The one operation `name` + `arg` names, with the profile left unset: the TUI
/// stamps the active profile, the headless path resolves the default.
fn parse_operation(name: &str, arg: &str) -> Result<Option<ContractsCommand>, SlashParseError> {
    match name {
        // One command over the knowledge store: the first token is the verb, and
        // a lone non-verb token stays a Show so today's `/contracts
        // catalog.schema.object` keeps working. `/contract` is the one-release
        // alias of this same arm.
        "contracts" | "contract" => verbs::parse_contracts(arg).map(Some),
        "remember" => {
            let spec = parse_remember(arg)?;
            Ok(Some(ContractsCommand::Remember {
                table: spec.table,
                kind: spec.kind,
                value: spec.value,
                column: spec.column,
                reason: spec.reason,
                profile: None,
            }))
        }
        "forget" => {
            // Exactly one token: the claim id.
            let tokens: Vec<&str> = arg.split_whitespace().collect();
            let id = tokens
                .first()
                .ok_or_else(|| SlashParseError(usage_forget()))?;
            if tokens.len() != 1 {
                return Err(SlashParseError(usage_forget()));
            }
            // The default reason matches the headless `forget` default so the
            // translated command equals the headless one with no --reason.
            Ok(Some(ContractsCommand::Forget {
                claim_id: id.to_string(),
                reason: ForgetReasonArg::UserRequest,
            }))
        }
        // The four deprecated spellings are the same verbs under the other name:
        // `/queue` is `/contracts queue`, `/approve-all` is `/contracts
        // confirm-all`. One grammar, so they cannot disagree.
        "queue" | "confirm" | "reject" | "approve-all" => verbs::parse_alias(name, arg).map(Some),
        _ => Ok(None),
    }
}

#[cfg(test)]
#[path = "command_tests.rs"]
mod tests;
