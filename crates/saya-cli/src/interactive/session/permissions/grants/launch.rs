//! The launch's `--allow` seeding: the same parse-and-gate, without a journal.

use super::allow::denied_payload;
use crate::commands::run::scopes::{self, Surface};
use saya_agent::SessionGrants;

/// Seeds the launch's `command:` tokens into the session's grant store:
/// the grammar stays the authority (each token parses on the session
/// surface), then the composition gate admits it — a token the composition
/// cannot carry is a launch usage error with its own reason, never a seeded
/// grant. Returns the seeded tokens verbatim, in stated order.
pub(crate) fn seed_launch_allow(
    tokens: &[String],
    composition: &crate::approval_facts::ApprovalFacts,
    grants: &SessionGrants,
) -> Result<Vec<String>, String> {
    let approved = scopes::parse(tokens, Surface::Session)?;
    let mut seeded = Vec::new();
    for token in &approved.tokens {
        if denied_payload(token, composition).is_some() {
            return Err(super::super::deny::allow_of_denied_refusal(token));
        }
        if let Some(refusal) = super::super::allow_refusal::composition_refusal(token, composition)
        {
            return Err(refusal);
        }
        grants.grant(token);
        seeded.push(token.clone());
    }
    Ok(seeded)
}
