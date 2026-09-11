//! `--budget KEY=VALUE` parsing: the CLI layer over the resolved `[jobs]`
//! defaults.
//!
//! The layering is RunSpec over `[jobs]` per dimension: every unset key
//! falls back to the config's resolved value, every declared key overrides
//! it. The environment is never read — a run must be reproducible from its
//! spec and config alone (plan G3). A zero ceiling is a typo, refused the
//! same way config resolution refuses one, never clamped.

use saya_types::{Budgets, is_name_shaped};
use std::time::Duration;

/// Parses the `--budget` tokens over the `[jobs]`-resolved base. Every key
/// is typed; an unknown key or a zero value is a usage error.
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
    Ok(budgets)
}

const KNOWN: &str = "known budgets: wall-clock=<seconds>, turns=<n>, tool-calls=<n>, \
                     tokens.<endpoint>=<n>";

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
