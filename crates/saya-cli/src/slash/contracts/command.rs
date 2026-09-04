//! The contract slash dispatcher: translates a slash command name plus its
//! argument tail into the matching `ContractsCommand`, or a payload-free usage
//! error. Each arm produces the same typed value the headless `saya contracts`
//! parser produces, so the slash and headless paths hand one value to the
//! shared `run_contracts` dispatcher — no second parsing, privacy decision, or
//! DTO mapping lives here.

use super::remember::parse_remember;
use crate::cli::{ContractsCommand, ForgetReasonArg, ReviewDecisionArg};
use crate::slash::SlashParseError;

/// Payload-free usage for the merged `/contracts` command. Never echoes the
/// untrusted tail — a two-token argument is a usage error, not a Show of either
/// token. The optional argument is what selects the operation: absent → list,
/// one token → show.
fn usage_contracts() -> String {
    "/contracts [catalog.schema.object]".into()
}

/// Payload-free usage for `/forget`.
fn usage_forget() -> String {
    "/forget <claim-id>".into()
}

/// Payload-free usage for `/queue`. The limit is optional and numeric.
fn usage_queue() -> String {
    "/queue [limit]".into()
}

/// Payload-free usage for the spec-D decide commands. One token: a stored
/// claim-id prefix (the `ki-xxxx` form `contracts list` abbreviates to). Never
/// echoes the untrusted prefix.
fn usage_decide() -> String {
    "/confirm|/reject|/use <claim-id-prefix>".into()
}

/// Payload-free usage for `/approve-all`. The optional tokens are
/// `--yes` and a numeric limit; anything else is a usage error.
fn usage_approve_all() -> String {
    "/approve-all [--yes] [limit]".into()
}

/// Parses the tail of a `/confirm`, `/reject`, or `/use` command: exactly one
/// token (the claim-id prefix). The decision is fixed by the command name.
/// Translates to `ContractsCommand::Decide` with `profile: None` — the TUI
/// stamps the active profile, the headless path resolves the default, exactly
/// as `/queue` does. A too-short prefix is not refused here: the resolve step
/// refuses `c` or empty with a typed message, so parsing stays shape-only.
fn parse_decide(
    _name: &str,
    arg: &str,
    decision: ReviewDecisionArg,
) -> Result<Option<ContractsCommand>, SlashParseError> {
    let tokens: Vec<&str> = arg.split_whitespace().collect();
    let prefix = tokens
        .first()
        .ok_or_else(|| SlashParseError(usage_decide()))?;
    if tokens.len() != 1 {
        return Err(SlashParseError(usage_decide()));
    }
    Ok(Some(ContractsCommand::Decide {
        prefix: prefix.to_string(),
        decision,
        profile: None,
    }))
}

/// Translates a slash command name + its argument tail into the matching
/// `ContractsCommand`, or a usage error. The argument is the raw text after the
/// command word (already trimmed of the leading `/name`).
pub(crate) fn parse_contract_command(
    name: &str,
    arg: &str,
) -> Result<Option<ContractsCommand>, SlashParseError> {
    match name {
        // one command, the optional argument selects the operation — what
        // the headless `saya contracts` CLI already does (`contracts list`,
        // `contracts show <t>`), and the same shape as `/queue [limit]`. No
        // argument → list every contract for the active profile; one token →
        // show that object's contract; two tokens → a payload-free usage error
        // (a qualified name is a single token). `/contract` is kept as a silent
        // alias of this same arm so the two names can no longer disagree on
        // what they do — see `contract_is_a_silent_alias_of_the_merged_command`
        // and the SPEC REVIEW for why it is an alias rather than removed. Both
        // pass `profile: None`: the TUI stamps the active profile, the
        // headless path resolves the default, exactly as before.
        "contracts" | "contract" => {
            let tokens: Vec<&str> = arg.split_whitespace().collect();
            match tokens.len() {
                0 => Ok(Some(ContractsCommand::List { profile: None })),
                1 => Ok(Some(ContractsCommand::Show {
                    table: tokens[0].to_string(),
                    profile: None,
                })),
                _ => Err(SlashParseError(usage_contracts())),
            }
        }
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
        "queue" => {
            // `/queue` with no args lists the active profile's candidates at the
            // default limit. A single optional positional number overrides the
            // limit. The slash path always uses the active profile — there is
            // no `--profile` form here, matching `/contracts` → `List`.
            let tokens: Vec<&str> = arg.split_whitespace().collect();
            let limit = match tokens.len() {
                0 => None,
                1 => Some(
                    tokens[0]
                        .parse::<usize>()
                        .map_err(|_| SlashParseError(usage_queue()))?,
                ),
                _ => return Err(SlashParseError(usage_queue())),
            };
            Ok(Some(ContractsCommand::Queue {
                profile: None,
                limit,
            }))
        }
        // Spec D: act on the claim the last turn showed, by a short stored
        // claim-id prefix rather than a 64-character id. The decision is fixed
        // by the command name; the prefix is the one positional. See
        // `parse_decide` and the §4 defence in the report.
        "confirm" => parse_decide(name, arg, ReviewDecisionArg::Confirm),
        "reject" => parse_decide(name, arg, ReviewDecisionArg::Reject),
        // the batch approve. `/approve-all` previews the queue and
        // approves nothing — deny by default, the same posture
        // `--non-interactive` gives approvals — and `/approve-all --yes`
        // approves. An optional number overrides the limit, matching
        // `/queue [limit]`. `profile: None`: the TUI stamps the active
        // profile, the headless path resolves the default.
        "approve-all" => {
            let mut yes = false;
            let mut limit = None;
            for token in arg.split_whitespace() {
                if token == "--yes" {
                    if yes {
                        return Err(SlashParseError(usage_approve_all()));
                    }
                    yes = true;
                } else if let Ok(parsed) = token.parse::<usize>() {
                    if limit.is_some() {
                        return Err(SlashParseError(usage_approve_all()));
                    }
                    limit = Some(parsed);
                } else {
                    return Err(SlashParseError(usage_approve_all()));
                }
            }
            Ok(Some(ContractsCommand::ApproveAll {
                profile: None,
                limit,
                yes,
            }))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
#[path = "command_tests.rs"]
mod tests;
