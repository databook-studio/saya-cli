//! The session grant commands' one behaviour: `/allow` seeds the session's
//! grant store, `/grants` lists it. The headless loop and the TUI dispatch
//! both call these, so the two surfaces render one operation — the words
//! the user typed are the record, and the parser is the grammar's only
//! authority.

use crate::commands::run::scopes::{self, Surface};
use saya_agent::{ApprovalChoice, ApprovalPolicy, SessionGrants, SessionPolicy};
use saya_store::{GrantSource, SessionJournal};

/// How a failed journal write is said — the one wording every journaling
/// site renders, so the two surfaces cannot drift. The consent always
/// stands; only the audit line is missing.
pub(crate) fn journal_warning(error: &saya_store::StoreError) -> String {
    format!(
        "warning: the session journal could not record this ({error}) — the grant stands, \
         but the audit line is missing"
    )
}

/// The `[s]` answer's one operation, shared by both prompt surfaces: record
/// the answer in the session's one policy, and journal a *new* grant there —
/// before the call it allowed runs, which is the best a per-call approval
/// surface can do and the ordering that makes the record meaningful. Allow-
/// once and deny record nothing. Returns whether a new grant landed, and —
/// when the journal write failed — the one warning wording: the consent
/// stands, the audit line is missing, and saying so is never optional.
pub(crate) fn record_prompt_answer(
    policy: &SessionPolicy,
    choice: &ApprovalChoice,
    journal: Option<&SessionJournal>,
) -> (bool, Option<String>) {
    let new_grant = policy.record(choice.clone());
    if !new_grant {
        return (false, None);
    }
    let ApprovalChoice::AllowSession { token } = choice else {
        return (true, None);
    };
    let Some(journal) = journal else {
        return (true, None);
    };
    match journal.granted(token, GrantSource::Prompt) {
        Ok(()) => (true, None),
        Err(error) => (true, Some(journal_warning(&error))),
    }
}

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
    // composition is the second gate.
    for token in &approved.tokens {
        if let Some(refusal) = super::allow_refusal::composition_refusal(token, composition) {
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
                warnings.push(journal_warning(&error));
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

/// `/grants`: the store's tokens verbatim, one per line, sorted, under a
/// header stating the lifetime, with a count — and an explicit empty state,
/// never a bare nothing. The words are the record, and they are the same
/// words the prompt offered. Under bypass the mode is stated **first**:
/// a count alone would read "nothing runs", when the truth is that every
/// call runs without asking and the store is never consulted. Every other
/// mode renders today's bytes exactly — the listing is the store's, and the
/// mode is the engine's own (`SessionPolicy::mode`).
pub(crate) fn listing(mode: ApprovalPolicy, grants: &SessionGrants) -> String {
    let mut out = String::new();
    if mode == ApprovalPolicy::Bypass {
        out.push_str("mode bypass: every call runs without asking; grants are not consulted\n");
    }
    let tokens = grants.tokens();
    out.push_str(&format!(
        "session grants (die with this session): {}",
        tokens.len()
    ));
    if tokens.is_empty() {
        out.push_str(
            "\n  (none — nothing pre-answers this session yet; /allow <scopes> \
             or answer [s] at an ask)",
        );
    } else {
        for token in tokens {
            out.push_str("\n  ");
            out.push_str(&token);
        }
    }
    out
}

/// The session grants a `/run --seed-grants` child receives: the tokens the
/// run's own parser accepts are forwarded into the child's `--allow`; the
/// rest are named, never silently dropped. The child's parser stays the
/// authority — the filter asks it, one token at a time. (`none` cannot sit
/// in a store: `/allow none` seeds nothing, and no ask ever offers it.)
pub(crate) struct RunSeed {
    /// The tokens forwarded as the child's `--allow`: exactly the ones the
    /// run surface parses.
    pub(crate) forwarded: Vec<String>,
    /// The tokens a run refuses — named to the user, never forwarded.
    pub(crate) dropped: Vec<String>,
}

pub(crate) fn run_seed(tokens: &[String]) -> RunSeed {
    let mut forwarded = Vec::new();
    let mut dropped = Vec::new();
    for token in tokens {
        match scopes::parse(std::slice::from_ref(token), Surface::Run) {
            Ok(_) => forwarded.push(token.clone()),
            Err(_) => dropped.push(token.clone()),
        }
    }
    RunSeed { forwarded, dropped }
}

/// The parent's own words about a seed request: what was forwarded, what was
/// not, and — over an empty store — that there was nothing to seed. Silence
/// about a drop would be exactly the lying-scope class the refusal list
/// exists to prevent.
pub(crate) fn seed_message(seed: &RunSeed) -> String {
    if seed.forwarded.is_empty() && seed.dropped.is_empty() {
        return "no session grants to seed — the run's --allow is yours to state".to_owned();
    }
    let mut lines = Vec::new();
    if !seed.forwarded.is_empty() {
        lines.push(format!(
            "seeded the run's --allow from this session's grants: {}",
            seed.forwarded.join(", ")
        ));
    }
    if !seed.dropped.is_empty() {
        lines.push(format!(
            "not forwarded — a run refuses these scopes (they would gate nothing there): {}",
            seed.dropped.join(", ")
        ));
    }
    lines.join("\n")
}
