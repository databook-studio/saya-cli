//! `config doctor`'s limits section: every resolved ceiling by its dotted key.
//!
//! The values come from the one resolved `[limits]` table; this module only
//! formats them. It is a display reader, not an enforcement adapter — no
//! engine receives a ceiling from here.

use crate::config::runtime::RuntimeConfig;
use saya_config::{BudgetLimit, ResolvedAnsweringLimits, ResolvedJobLimits, ResolvedRunLimits};
use saya_types::CodingBudgetMode;

/// One line per ceiling, in table order. `unlimited` is the resolved answer,
/// not an absent line: "no ceiling" and "not reported" are different facts.
pub(crate) fn lines(runtime: &RuntimeConfig) -> Vec<String> {
    let limits = &runtime.resolved.limits;
    let mut lines = Vec::new();
    push_answering(&mut lines, "limits.session", limits.session);
    push_answering(&mut lines, "limits.ask", limits.ask);
    push_run(&mut lines, &limits.run);
    push_job(&mut lines, &limits.job);
    lines
}

fn push_answering(lines: &mut Vec<String>, prefix: &str, limits: ResolvedAnsweringLimits) {
    for (name, value) in [
        ("requests", limits.requests),
        ("tool_calls", limits.tool_calls),
        ("elapsed_seconds", limits.elapsed_seconds),
        ("known_tokens", limits.known_tokens),
    ] {
        lines.push(format!("{prefix}.{name}: {}", limit(value)));
    }
}

fn push_run(lines: &mut Vec<String>, limits: &ResolvedRunLimits) {
    for (name, value) in [
        ("wall_clock_seconds", limits.wall_clock_seconds),
        ("turns", limits.turns),
        ("tool_calls", limits.tool_calls),
    ] {
        lines.push(format!("limits.run.{name}: {}", limit(value)));
    }
    lines.push(format!(
        "limits.run.tokens_per_endpoint: {}",
        tokens(&limits.tokens_per_endpoint)
    ));
    lines.push(format!(
        "limits.run.no_progress_turns: {}",
        limits.no_progress_turns
    ));
    lines.push(format!("limits.run.repeat_limit: {}", limits.repeat_limit));
}

fn push_job(lines: &mut Vec<String>, limits: &ResolvedJobLimits) {
    lines.push(format!(
        "limits.job.budget_mode: {}",
        mode(limits.budget_mode)
    ));
    for (name, value) in [
        ("logical_requests", limits.logical_requests()),
        ("transport_attempts", limits.transport_attempts()),
        ("tool_calls", limits.tool_calls()),
        ("known_tokens", limits.known_tokens()),
        ("job_tokens", limits.job_tokens()),
        ("elapsed_seconds", limits.elapsed_seconds()),
    ] {
        lines.push(format!("limits.job.{name}: {}", limit(value)));
    }
    lines.push(format!(
        "limits.job.wall_clock_seconds: {}",
        match limits.wall_clock_seconds {
            BudgetLimit::Unlimited => "unlimited (no wall-clock guard)".to_owned(),
            value => limit(value),
        }
    ));
    lines.push(format!(
        "limits.job.no_progress_turns: {}{}",
        limits.no_progress_turns,
        if limits.no_progress_turns > 0 {
            " (the guard that ends a stuck job)"
        } else {
            ""
        }
    ));
    lines.push(format!("limits.job.repeat_limit: {}", limits.repeat_limit));
    lines.push(format!(
        "limits.job.max_cost_usd: {}",
        limits.max_cost_micros.map_or_else(
            || "unlimited".to_owned(),
            |micros| format!("{:.6}", micros as f64 / 1_000_000.0)
        )
    ));
}

fn limit(value: BudgetLimit) -> String {
    value
        .value()
        .map_or_else(|| "unlimited".to_owned(), |value| value.to_string())
}

fn tokens(endpoints: &std::collections::BTreeMap<String, u64>) -> String {
    if endpoints.is_empty() {
        return "none".to_owned();
    }
    endpoints
        .iter()
        .map(|(name, tokens)| format!("{name}={tokens}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn mode(mode: CodingBudgetMode) -> &'static str {
    match mode {
        CodingBudgetMode::Bounded => "bounded",
        CodingBudgetMode::Uncapped => "uncapped",
        _ => "unknown",
    }
}
