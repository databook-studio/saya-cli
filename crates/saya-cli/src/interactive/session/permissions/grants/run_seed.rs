//! The session grants a `/run --seed-grants` child receives: what the run's
//! own parser forwards, and the parent's words about the seed.

use crate::commands::run::scopes::{self, Surface};

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
