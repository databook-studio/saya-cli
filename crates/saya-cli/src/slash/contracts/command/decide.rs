//! The decide arm: `/confirm`, `/reject`, and `/use` by claim-id prefix.

use crate::cli::{ContractsCommand, ReviewDecisionArg};
use crate::slash::SlashParseError;

/// Payload-free usage for the spec-D decide commands. One token: a stored
/// claim-id prefix (the `ki-xxxx` form `contracts list` abbreviates to). Never
/// echoes the untrusted prefix.
fn usage_decide() -> String {
    "/confirm|/reject|/use <claim-id-prefix>".into()
}

/// Parses the tail of a `/confirm`, `/reject`, or `/use` command: exactly one
/// token (the claim-id prefix). The decision is fixed by the command name.
/// Translates to `ContractsCommand::Decide` with `profile: None` — the TUI
/// stamps the active profile, the headless path resolves the default, exactly
/// as `/queue` does. A too-short prefix is not refused here: the resolve step
/// refuses `c` or empty with a typed message, so parsing stays shape-only.
pub(super) fn parse_decide(
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
