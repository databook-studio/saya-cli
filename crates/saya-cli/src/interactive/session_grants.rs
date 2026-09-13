//! The session grant commands' one behaviour: `/allow` seeds the session's
//! grant store, `/grants` lists it. The headless loop and the TUI dispatch
//! both call these, so the two surfaces render one operation — the words
//! the user typed are the record, and the parser is the grammar's only
//! authority.

use crate::commands::run::scopes::{self, Surface};
use saya_agent::{ApprovalPolicy, SessionGrants};

/// `/allow <scopes…>`: parse the tokens on the session surface, seed the
/// store with the stated tokens verbatim, and say what was seeded.
///
/// `none` keeps its grammar meaning — the empty approval, alone — and
/// seeds nothing, saying so. It is not a revoke: the store is additive
/// only, so whatever the session already holds stays held. A refused scope
/// is a usage error and seeds nothing.
pub(crate) fn allow(tokens: &[String], grants: &SessionGrants) -> Result<String, String> {
    let approved = scopes::parse(tokens, Surface::Session)?;
    if approved.tokens.iter().any(|token| token == "none") {
        return Ok(
            "`none` states the empty approval: nothing seeded, nothing revoked — \
                   the store keeps whatever this session already holds."
                .to_owned(),
        );
    }
    let mut seeded = Vec::new();
    let mut already = Vec::new();
    for token in &approved.tokens {
        if grants.grant(token) {
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
