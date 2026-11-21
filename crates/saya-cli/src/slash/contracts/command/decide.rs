//! The decide arm: `/contracts confirm|reject <claim-id-prefix>`, and the
//! deprecated `/confirm` and `/reject` spellings that name the same verbs.

use crate::cli::{ContractsCommand, ReviewDecisionArg};
use crate::slash::SlashParseError;

/// Parses the tail of a decide spelling: exactly one token (the claim-id
/// prefix). The decision is fixed by the verb. `written` is the spelling as
/// typed — `contracts confirm` or `confirm` — so the usage line names the form
/// the user actually typed, and never echoes the untrusted prefix.
pub(super) fn parse_decide(
    written: &str,
    rest: &[&str],
    decision: ReviewDecisionArg,
) -> Result<ContractsCommand, SlashParseError> {
    let usage = || SlashParseError(format!("/{written} <claim-id-prefix>"));
    let [prefix] = rest else {
        return Err(usage());
    };
    // A too-short prefix is not refused here: the resolve step refuses `c` or
    // empty with a typed message, so parsing stays shape-only.
    Ok(ContractsCommand::Decide {
        prefix: (*prefix).to_string(),
        decision,
        profile: None,
    })
}
