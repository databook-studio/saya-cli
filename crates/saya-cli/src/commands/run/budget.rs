//! `--budget KEY=VALUE` parsing: the CLI layer over the resolved `[jobs]`
//! defaults.
//!
//! The layering is RunSpec over `[jobs]` per dimension: every unset key
//! falls back to the config's resolved value, every declared key overrides
//! it. The environment is never read — a run must be reproducible from its
//! spec and config alone (plan G3). A zero ceiling is a typo, refused the
//! same way config resolution refuses one, never clamped.

use saya_config::ORCHESTRATOR_ROLE;
use saya_types::{Budgets, is_name_shaped};
use std::time::Duration;

/// Parses the `--budget` tokens over the `[jobs]`-resolved base. Every key
/// is typed; an unknown key or a zero value is a usage error, and a token
/// ceiling naming a role other than the orchestrator is refused like the
/// unwired scopes (`scopes.rs`).
pub(super) fn parse(tokens: &[String], base: &Budgets) -> Result<Budgets, String> {
    let mut budgets = base.clone();
    for token in tokens {
        let Some((key, value)) = token.split_once('=') else {
            return Err(format!("budget `{token}` must be KEY=VALUE; {KNOWN}"));
        };
        let number = parse_positive(key, value)?;
        match key {
            "wall-clock" => budgets.wall_clock = Some(Duration::from_secs(number)),
            "turns" => budgets.turns = Some(number),
            "tool-calls" => budgets.tool_calls = Some(number),
            other => {
                let Some(endpoint) = other.strip_prefix("tokens.") else {
                    return Err(format!("unknown budget key `{other}`; {KNOWN}"));
                };
                if !is_name_shaped(endpoint) {
                    return Err(format!(
                        "budget key `{other}` is not a run-scoped endpoint name; {KNOWN}"
                    ));
                }
                budgets
                    .tokens_per_endpoint
                    .insert(endpoint.to_string(), number);
            }
        }
    }
    // Refusal by the scopes' own discipline (`scopes.rs`): a ceiling for a
    // role no episode calls does not bind nothing — `token_ceiling` takes
    // the tightest declared ceiling and binds it to the one endpoint every
    // episode calls, so `tokens.analyst=100` would silently cap the whole
    // run at 100. The check runs over the merged map, so a leftover
    // `[jobs] tokens_per_endpoint` entry for an unwired role is refused at
    // start too, not quietly tightened onto the orchestrator.
    if let Some(role) = budgets
        .tokens_per_endpoint
        .keys()
        .find(|role| role.as_str() != ORCHESTRATOR_ROLE)
    {
        return Err(format!(
            "budget key `tokens.{role}` is not available yet: every episode calls the \
             `{ORCHESTRATOR_ROLE}` endpoint, and the tightest declared ceiling binds it, so \
             a ceiling for a role that never runs would silently cap the whole run. Re-run \
             without it."
        ));
    }
    Ok(budgets)
}

const KNOWN: &str = "known budgets: wall-clock=<seconds>, turns=<n>, tool-calls=<n>, \
                     tokens.orchestrator=<n>";

/// Every ceiling is at least one: a zero stops a run before it starts, which
/// as a declaration is a typo, not an intent — refused, never clamped.
fn parse_positive(key: &str, value: &str) -> Result<u64, String> {
    let parsed: u64 = value
        .parse()
        .map_err(|_| format!("budget `{key}` needs a whole number, got `{value}`"))?;
    if parsed == 0 {
        return Err(format!(
            "budget `{key}=0` is a typo, not an intent; use 1 or more"
        ));
    }
    Ok(parsed)
}

/// The token ceiling the engine enforces, taken from the run's declared
/// per-endpoint budget.
///
/// Every episode currently calls the single orchestrator endpoint, so there
/// is one bucket and its ceiling is the run's. `parse` refuses every other
/// key, so a spec built through this surface carries at most that one key —
/// but a spec reaches this function through plain serde on resume
/// (`files.rs` loads without re-parsing budgets), and such a map may carry
/// several keys, so the tightest declared ceiling binds: stopping at the
/// smallest is the fail-safe reading, never the generous one.
pub(super) fn token_ceiling(budgets: &saya_types::Budgets) -> Option<u64> {
    if budgets.tokens_per_endpoint.is_empty() {
        return None;
    }
    budgets.tokens_per_endpoint.values().copied().min()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// A ceiling for a role that never runs is not inert: the tightest
    /// declared ceiling binds the orchestrator, so `tokens.analyst=100`
    /// would cap the whole run at 100 tokens. Refused at parse time, the
    /// same discipline the unwired scopes got (`scopes.rs`).
    #[test]
    fn a_ceiling_for_a_role_no_episode_calls_is_refused() {
        let error = parse(&["tokens.analyst=100".to_string()], &Budgets::default())
            .expect_err("a non-orchestrator ceiling must refuse");
        assert!(error.contains("tokens.analyst"), "got: {error}");
        assert!(error.contains("orchestrator"), "got: {error}");
    }

    /// The refusal reaches the `[jobs]`-resolved base too: a leftover
    /// `tokens_per_endpoint` entry for an unwired role is refused at start
    /// rather than silently tightened onto the orchestrator.
    #[test]
    fn a_base_ceiling_for_an_unwired_role_is_refused_too() {
        let mut base = Budgets::default();
        base.tokens_per_endpoint
            .insert("analyst".to_string(), 1_000);
        let error = parse(&[], &base).expect_err("a non-orchestrator base key must refuse");
        assert!(error.contains("tokens.analyst"), "got: {error}");
    }

    /// The orchestrator's own ceiling parses and overrides the `[jobs]`
    /// base, per the layering rule every dimension follows.
    #[test]
    fn the_orchestrator_ceiling_parses_and_overrides_the_base() {
        let mut base = Budgets::default();
        base.tokens_per_endpoint
            .insert(ORCHESTRATOR_ROLE.to_string(), 2_000);
        let budgets = parse(&["tokens.orchestrator=500".to_string()], &base).unwrap();
        assert_eq!(
            budgets.tokens_per_endpoint.get(ORCHESTRATOR_ROLE),
            Some(&500)
        );
    }

    /// min() binds the tightest declared ceiling when a map carries several
    /// keys. The parser refuses all but the orchestrator's, but this
    /// function also sees specs loaded with plain serde on resume
    /// (`files.rs`), so the fail-safe reading needs its own pin.
    #[test]
    fn the_tightest_declared_ceiling_binds_when_several_survive() {
        let mut map = BTreeMap::new();
        map.insert(ORCHESTRATOR_ROLE.to_string(), 5_000_u64);
        map.insert("other".to_string(), 100_u64);
        let mut budgets = Budgets::default();
        budgets.tokens_per_endpoint = map;
        assert_eq!(
            token_ceiling(&budgets),
            Some(100),
            "the smallest declared ceiling binds, never the generous one"
        );
    }
}
