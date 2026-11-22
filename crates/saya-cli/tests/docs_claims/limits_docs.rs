//! `docs/configuration.md`'s `[limits]` pages are executable claims:
//!
//! - every key a `[limits…]` example states is a key the config reads — the
//!   `[limits]` tables refuse an unknown field, so a typo in the page fails
//!   here rather than sending a reader to a setting that does nothing;
//! - the defaults the page states are the resolver's own, built from
//!   `ResolvedLimits::default()` rather than copied beside it.

use saya_config::{ConfigFile, ResolvedLimits};

const PAGE: &str = include_str!("../../../../docs/configuration.md");

/// Every fenced ```toml block of the page that states a `[limits…]` title.
fn limits_blocks(page: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in page.lines() {
        if let Some(block) = current.as_mut() {
            if line.trim() == "```" {
                blocks.push(current.take().unwrap_or_default());
            } else {
                block.push_str(line);
                block.push('\n');
            }
            continue;
        }
        if line.trim() == "```toml" {
            current = Some(String::new());
        }
    }
    blocks
        .into_iter()
        .filter(|block| block.lines().any(|line| line.starts_with("[limits")))
        .collect()
}

/// Thousands separators, as the page writes its numbers.
fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, digit) in digits.char_indices() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// A ceiling the page must state as this number.
fn count(limit: saya_config::BudgetLimit, key: &str) -> String {
    let value = limit
        .value()
        .unwrap_or_else(|| panic!("{key} is a finite default, so the page states a number"));
    grouped(value)
}

#[test]
fn every_limits_key_the_page_states_is_a_key_the_config_reads() {
    let blocks = limits_blocks(PAGE);
    assert!(
        blocks.len() >= 3,
        "the page must still show [limits] examples; found {}",
        blocks.len()
    );
    for block in &blocks {
        if let Err(error) = toml::from_str::<ConfigFile>(block) {
            panic!(
                "docs/configuration.md states a [limits] key the config does not read:\n{block}\n{error}"
            );
        }
    }
}

#[test]
fn the_limits_at_a_glance_table_states_the_resolved_defaults() {
    let limits = ResolvedLimits::default();
    let database = format!(
        "{} requests, {} tool calls, {} s, {} known tokens",
        count(limits.database.requests, "limits.database.requests"),
        count(limits.database.tool_calls, "limits.database.tool_calls"),
        count(
            limits.database.elapsed_seconds,
            "limits.database.elapsed_seconds"
        ),
        count(limits.database.known_tokens, "limits.database.known_tokens"),
    );
    assert!(
        PAGE.contains(&database),
        "the limits table must state the database budget's defaults, `{database}`"
    );
    let job = format!(
        "bounded: {} logical requests, {} transport attempts, {} tool calls, a {} s deadline, \
         unlimited per-loop tokens; no job-wide token ceiling",
        count(limits.job.logical_requests(), "limits.job.logical_requests"),
        count(
            limits.job.transport_attempts(),
            "limits.job.transport_attempts"
        ),
        count(limits.job.tool_calls(), "limits.job.tool_calls"),
        count(limits.job.elapsed_seconds(), "limits.job.elapsed_seconds"),
    );
    assert!(
        PAGE.contains(&job),
        "the limits table must state the job's bounded defaults, `{job}`"
    );
    assert!(
        limits.job.known_tokens().value().is_none() && limits.job.job_tokens().value().is_none(),
        "the job's token ceilings are unlimited by default, so the page says so"
    );
    assert!(
        PAGE.contains(&format!(
            "no_progress_turns = {}",
            limits.job.no_progress_turns
        )) && PAGE.contains(&format!("repeat_limit = {}", limits.job.repeat_limit)),
        "the page states the two progress guards at their defaults"
    );
}
