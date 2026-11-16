use super::{AnsweringLimitsFile, JobLimitsFile, LimitsFile, RunLimitsFile};
use crate::{SettingSource, provenance::Recorder};

/// Merges one layer's `[limits]` table over the base, sub-table by sub-table
/// and key by key: a higher layer overrides exactly the dimensions it states.
/// The answering and coding sub-tables are protected, so those only ever see
/// trusted layers — the revert in `layers/protected/limits.rs` is what keeps an
/// untrusted one out. `[limits.run]` is ordinary and merges like any other
/// project-settable table.
pub(crate) fn merge(
    base: &mut LimitsFile,
    layer: &LimitsFile,
    recorder: &mut Recorder,
    source: impl Fn(&str) -> SettingSource,
) {
    merge_answering(
        &mut base.session,
        &layer.session,
        recorder,
        &source,
        "limits.session",
    );
    merge_answering(&mut base.ask, &layer.ask, recorder, &source, "limits.ask");
    merge_run(&mut base.run, &layer.run, recorder, &source);
    merge_job(&mut base.job, &layer.job, recorder, &source);
}

/// The coding sub-table's own entry point, so the CLI layer can attribute each
/// `--coding-max-*` flag to the key it states rather than to the section.
pub(crate) fn merge_job(
    base: &mut JobLimitsFile,
    layer: &JobLimitsFile,
    recorder: &mut Recorder,
    source: impl Fn(&str) -> SettingSource,
) {
    if let Some(mode) = layer.budget_mode {
        base.budget_mode = Some(mode);
        recorder.record("limits.job.budget_mode", source("budget_mode"));
    }
    for (name, layer_value, base_value) in [
        (
            "logical_requests",
            &layer.logical_requests,
            &mut base.logical_requests,
        ),
        (
            "transport_attempts",
            &layer.transport_attempts,
            &mut base.transport_attempts,
        ),
        ("tool_calls", &layer.tool_calls, &mut base.tool_calls),
        ("known_tokens", &layer.known_tokens, &mut base.known_tokens),
        ("job_tokens", &layer.job_tokens, &mut base.job_tokens),
        (
            "elapsed_seconds",
            &layer.elapsed_seconds,
            &mut base.elapsed_seconds,
        ),
        (
            "no_progress_turns",
            &layer.no_progress_turns,
            &mut base.no_progress_turns,
        ),
        ("repeat_limit", &layer.repeat_limit, &mut base.repeat_limit),
        (
            "wall_clock_seconds",
            &layer.wall_clock_seconds,
            &mut base.wall_clock_seconds,
        ),
        ("max_cost_usd", &layer.max_cost_usd, &mut base.max_cost_usd),
    ] {
        if let Some(value) = layer_value {
            *base_value = Some(value.clone());
            recorder.record(&format!("limits.job.{name}"), source(name));
        }
    }
}

fn merge_run(
    base: &mut RunLimitsFile,
    layer: &RunLimitsFile,
    recorder: &mut Recorder,
    source: &impl Fn(&str) -> SettingSource,
) {
    for (name, layer_value, base_value) in [
        (
            "wall_clock_seconds",
            &layer.wall_clock_seconds,
            &mut base.wall_clock_seconds,
        ),
        ("turns", &layer.turns, &mut base.turns),
        ("tool_calls", &layer.tool_calls, &mut base.tool_calls),
        (
            "no_progress_turns",
            &layer.no_progress_turns,
            &mut base.no_progress_turns,
        ),
        ("repeat_limit", &layer.repeat_limit, &mut base.repeat_limit),
    ] {
        if let Some(value) = layer_value {
            *base_value = Some(value.clone());
            recorder.record(&format!("limits.run.{name}"), source(name));
        }
    }
    // The token map is a map rather than a dimension, so it is stated whole or
    // not at all, like the per-endpoint shape it mirrors.
    if let Some(value) = &layer.tokens_per_endpoint {
        base.tokens_per_endpoint = Some(value.clone());
        recorder.record(
            "limits.run.tokens_per_endpoint",
            source("tokens_per_endpoint"),
        );
    }
}

fn merge_answering(
    base: &mut AnsweringLimitsFile,
    layer: &AnsweringLimitsFile,
    recorder: &mut Recorder,
    source: &impl Fn(&str) -> SettingSource,
    prefix: &str,
) {
    for (name, layer_value, base_value) in [
        ("requests", &layer.requests, &mut base.requests),
        ("tool_calls", &layer.tool_calls, &mut base.tool_calls),
        (
            "elapsed_seconds",
            &layer.elapsed_seconds,
            &mut base.elapsed_seconds,
        ),
        ("known_tokens", &layer.known_tokens, &mut base.known_tokens),
    ] {
        if let Some(value) = layer_value {
            *base_value = Some(value.clone());
            recorder.record(&format!("{prefix}.{name}"), source(name));
        }
    }
}
