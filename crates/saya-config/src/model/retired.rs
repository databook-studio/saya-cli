//! Settings this release refuses because it does not define them. Each is
//! refused with its removal stated and the fix named, instead of serde's bare
//! `unknown field` error — a removed setting is not a typo, and the config that
//! carries it was written by a binary that meant something by it.
//!
//! The table lives beside the `ConfigFile` it guards rather than in it: it grows
//! with every retirement and would otherwise crowd the type it protects.

const RETIRED_KEYS: &[(&str, &str, &str)] = &[
    (
        "coding",
        "enabled",
        "`[coding] enabled` was removed; coding is always available. Delete the line.",
    ),
    (
        "coding",
        "check_profiles",
        "[coding] check_profiles was removed; checks no longer run. Delete the section.",
    ),
    (
        "coding",
        "max_concurrent_commands",
        "[coding] max_concurrent_commands was removed; owner commands no longer run. \
         Delete the key.",
    ),
    (
        "jobs",
        "runner",
        "[jobs.runner] was removed; run_program no longer exists. Move program_dir to \
         [job] program_dir and delete the section.",
    ),
    (
        "jobs",
        "interpreter",
        "[jobs.interpreter] was removed; run_program no longer exists. Delete the section.",
    ),
    (
        "coding",
        "max_children",
        "[coding] max_children was moved: helper agent types and the bounds they run under \
         live in the top-level [agents] table. Write agents.max_children there instead.",
    ),
    (
        "coding",
        "max_concurrent_model_requests",
        "[coding] max_concurrent_model_requests was moved: helper agent types and the bounds \
         they run under live in the top-level [agents] table. Write \
         agents.max_concurrent_model_requests there instead.",
    ),
    (
        "coding",
        "agents",
        "[coding.agents] was moved: helper agent types live in the top-level [agents] table. \
         Move the section to [agents].",
    ),
    (
        "coding",
        "budget_mode",
        "[coding] budget_mode was moved: write limits.job.budget_mode instead.",
    ),
    (
        "coding",
        "max_logical_model_requests",
        "[coding] max_logical_model_requests was moved: write \
         limits.job.logical_requests instead.",
    ),
    (
        "coding",
        "max_physical_transport_attempts",
        "[coding] max_physical_transport_attempts was moved: write \
         limits.job.transport_attempts instead.",
    ),
    (
        "coding",
        "max_tool_calls",
        "[coding] max_tool_calls was moved: write limits.job.tool_calls instead.",
    ),
    (
        "coding",
        "max_known_tokens",
        "[coding] max_known_tokens was moved: write limits.job.known_tokens instead.",
    ),
    (
        "coding",
        "max_job_tokens",
        "[coding] max_job_tokens was moved: write limits.job.job_tokens instead.",
    ),
    (
        "coding",
        "max_elapsed_seconds",
        "[coding] max_elapsed_seconds was moved: write limits.job.elapsed_seconds instead.",
    ),
    (
        "coding",
        "shell",
        "[coding] shell was removed: the [tools] allow list is the one switch for what a \
         surface advertises. Remove `shell` from [tools] allow instead.",
    ),
    (
        "coding",
        "program_dir",
        "[coding] program_dir was moved: the job host's own settings live in the [job] \
         table. Write job.program_dir there instead.",
    ),
    (
        "coding",
        "extensions",
        "[coding.extensions] was moved: write [job.extensions] instead.",
    ),
    (
        "coding",
        "automatic_lsp_profile",
        "[coding] automatic_lsp_profile was moved: write job.automatic_lsp_profile instead.",
    ),
    (
        "ai",
        "investigation_max_logical_answering_requests",
        "[ai] investigation_max_logical_answering_requests was moved: write \
         limits.session.requests instead.",
    ),
    (
        "ai",
        "investigation_max_requested_tool_calls",
        "[ai] investigation_max_requested_tool_calls was moved: write \
         limits.session.tool_calls instead.",
    ),
    (
        "ai",
        "investigation_max_elapsed_seconds",
        "[ai] investigation_max_elapsed_seconds was moved: write \
         limits.session.elapsed_seconds instead.",
    ),
    (
        "ai",
        "investigation_max_known_reported_tokens",
        "[ai] investigation_max_known_reported_tokens was moved: write \
         limits.session.known_tokens instead.",
    ),
    (
        "jobs",
        "wall_clock_seconds",
        "[jobs] wall_clock_seconds was moved: write limits.run.wall_clock_seconds instead.",
    ),
    (
        "jobs",
        "tokens_per_endpoint",
        "[jobs] tokens_per_endpoint was moved: write limits.run.tokens_per_endpoint instead.",
    ),
    (
        "jobs",
        "turns",
        "[jobs] turns was moved: write limits.run.turns instead.",
    ),
    (
        "jobs",
        "tool_calls",
        "[jobs] tool_calls was moved: write limits.run.tool_calls instead.",
    ),
    (
        "run",
        "max_iterations",
        "[run] max_iterations was removed; `limits.run.turns` is the run's turn ceiling. \
         Delete the key.",
    ),
];

/// Whole sections this release refuses because it does not define them.
/// A key inside one with a row of its own in [`RETIRED_KEYS`] reports that
/// row; every other key reports the section's own move, so a renamed table
/// leaves nothing behind that reads as a typo.
const RETIRED_SECTIONS: &[(&str, &str)] = &[(
    "coding",
    "[coding] was renamed: the job host's own settings live in the [job] table, and its \
     ceilings in [limits.job]. Move the section.",
)];

/// Names the first connection profile whose `type` this release removed, if
/// any. A profile written by an older binary is not a typo: serde would report
/// a bare "unknown variant" without saying which profile carried it, so the
/// name and the removal are read from the text.
pub(super) fn removed_profile_hint(raw: &str) -> Option<String> {
    let value: toml::Value = toml::from_str(raw).ok()?;
    let profiles = value.as_table()?.get("profiles")?.as_table()?;
    for (name, profile) in profiles {
        let Some(kind) = profile.as_table().and_then(|table| table.get("type")) else {
            continue;
        };
        if kind.as_str() == Some("duckdb") {
            return Some(format!(
                "connection profile {name:?} uses type = \"duckdb\", which was removed: analyse \
                 DuckDB files with the duckdb CLI through the shell, or connect the data through \
                 another supported database."
            ));
        }
    }
    None
}

/// Names the first retired key the raw TOML still sets, if any. Like
/// [`crate::ConfigFile::from_toml`]'s other shape hints this reads the source
/// directly: the typed parse fails with a generic message, so the specific one
/// has to come from the text.
pub(super) fn removed_key_hint(raw: &str) -> Option<String> {
    let value: toml::Value = toml::from_str(raw).ok()?;
    let table = value.as_table()?;
    for (section, key, message) in RETIRED_KEYS {
        if table
            .get(*section)
            .and_then(toml::Value::as_table)
            .is_some_and(|section| section.contains_key(*key))
        {
            return Some((*message).to_owned());
        }
    }
    for (section, message) in RETIRED_SECTIONS {
        if table.contains_key(*section) {
            return Some((*message).to_owned());
        }
    }
    None
}
