//! `/allow <scopes…>`: parse, gate on the composition, seed the store.

use crate::commands::run::scopes::{self, Surface};
use saya_agent::SessionGrants;
use saya_store::{GrantSource, SessionJournal};

/// `/allow <scopes…>`: parse the tokens on the session surface, refuse the
/// ones this session's composition cannot carry, seed the store with the
/// stated tokens verbatim, and say what was seeded.
///
/// `composition` is what this session composed — the same facts the
/// approval prompts state — and a token that gates nothing in it is a
/// usage error with its own reason, never a seeded grant: a grant the
/// composition cannot honour would pre-answer asks into refusals and
/// silence every ask that would say something is wrong (U8).
///
/// `none` keeps its grammar meaning — the empty approval, alone — and
/// seeds nothing, saying so. It is not a revoke: the store is additive
/// only, so whatever the session already holds stays held. A refused scope
/// is a usage error and seeds nothing.
///
/// Every newly seeded token is journalled once (`source: "seed"`), before
/// anything runs under it — the store's own new-grant answer is the
/// journal-once hook. A failed journal write changes no grant; its warning
/// is folded into the message, never silent.
pub(crate) fn allow(
    tokens: &[String],
    composition: &crate::approval_facts::ApprovalFacts,
    grants: &SessionGrants,
    journal: &SessionJournal,
) -> Result<String, String> {
    let approved = scopes::parse(tokens, Surface::Session)?;
    if approved.tokens.iter().any(|token| token == "none") {
        return Ok(
            "`none` states the empty approval: nothing seeded, nothing revoked — \
                   the store keeps whatever this session already holds."
                .to_owned(),
        );
    }
    // A token the composition cannot carry is a usage error with its own
    // reason, checked over the whole statement before anything seeds — a
    // refusal never half-seeds. The grammar accepted the word; the
    // composition is the second gate. A token naming a denied program
    // refuses first: a grant cannot override the deny list.
    for token in &approved.tokens {
        if denied_payload(token, composition).is_some() {
            return Err(super::super::deny::allow_of_denied_refusal(token));
        }
        if let Some(refusal) = super::super::allow_refusal::composition_refusal(token, composition)
        {
            return Err(refusal);
        }
    }
    let mut seeded = Vec::new();
    let mut already = Vec::new();
    let mut warnings = Vec::new();
    for token in &approved.tokens {
        if grants.grant(token) {
            // Journal once, at the moment of seeding — the same act that
            // puts the token in the store.
            if let Err(error) = journal.granted(token, GrantSource::Seed) {
                warnings.push(super::journal_warning(&error));
            }
            seeded.push(token.clone());
        } else {
            already.push(token.clone());
        }
    }
    let mut message = String::new();
    if !seeded.is_empty() {
        message.push_str(&format!(
            "granted for this session (dies with it): {}",
            seeded.join(", ")
        ));
    }
    if !already.is_empty() {
        if !message.is_empty() {
            message.push('\n');
        }
        message.push_str(&format!(
            "already granted (nothing changed): {}",
            already.join(", ")
        ));
    }
    for warning in warnings {
        if !message.is_empty() {
            message.push('\n');
        }
        message.push_str(&warning);
    }
    Ok(message)
}

/// The denied program a token's payload names, when the session's deny
/// list carries it: the payload of `command:`, `runner:`, and
/// `interpreter:` tokens is the program the door would exec.
pub(super) fn denied_payload(
    token: &str,
    composition: &crate::approval_facts::ApprovalFacts,
) -> Option<String> {
    let payload = token
        .strip_prefix("command:")
        .or_else(|| token.strip_prefix("runner:"))
        .or_else(|| token.strip_prefix("interpreter:"))?;
    composition
        .denied_programs
        .iter()
        .any(|denied| denied == payload)
        .then(|| payload.to_owned())
}
