//! The built-in defaults fill: after resolution, every user-facing key with
//! no recorded origin reports the `Default` layer.

use std::collections::BTreeMap;

use super::source::{SettingOrigin, SettingProvenance, SettingSource};
use crate::endpoints::ResolvedEndpoint;

/// The dotted config keys `redacted_diagnostics` reports (the settings
/// `config show`/`doctor` display), plus `instructions.*` and the
/// host/session-command presence keys. The drift-guard test in this module
/// proves every key here is one the merge funnels record.
pub(crate) const USER_FACING: &[&str] = &[
    "profile_name",
    "ai.provider",
    "ai.model",
    "ai.base_url",
    "ai.api_key",
    "ai.allow_data_sharing",
    "ai.temperature",
    "ai.timeout_seconds",
    "ai.idle_timeout_seconds",
    "ai.max_output_tokens",
    "ai.retry_delays_ms",
    "ai.context_byte_budget",
    "ai.context_window_tokens",
    "ai.show_thinking",
    "ai.compaction",
    "run.read_only",
    "run.max_rows",
    "run.max_iterations",
    "run.candidates",
    "run.query_timeout_seconds",
    "jobs.wall_clock_seconds",
    "jobs.tokens_per_endpoint",
    "jobs.turns",
    "jobs.tool_calls",
    "jobs.fetch",
    "jobs.runner",
    "output.format",
    "output.color",
    "ui.theme",
    "memory.mode",
    "memory.max_contracts",
    "memory.max_claims_per_contract",
    "memory.max_context_bytes",
    "instructions.project",
    "instructions.personal",
    "instructions.project_max_bytes",
    "instructions.personal_max_bytes",
    "instructions.total_max_bytes",
    "host_commands.pass_env",
    "host_commands.timeout_seconds",
    "session_commands.deny",
];

/// Coding defaults use the same value-free provenance fill as other resolved
/// settings.
const CODING: &[&str] = &[
    "coding.enabled",
    "coding.max_children",
    "coding.max_concurrent_model_requests",
    "coding.max_concurrent_commands",
    "coding.max_logical_model_requests",
    "coding.max_physical_transport_attempts",
    "coding.max_tool_calls",
    "coding.max_known_tokens",
    "coding.max_elapsed_seconds",
];

/// After resolution, every user-facing key with no recorded origin is a
/// built-in default: `resolve` applied it without any layer stating it.
/// Endpoint fields are keyed exactly as `revert_untrusted` names them, so
/// one key shape serves recording, rollback, and the defaults fill.
pub(crate) fn apply_user_facing_defaults(
    provenance: &mut SettingProvenance,
    endpoints: &BTreeMap<String, ResolvedEndpoint>,
) {
    let default = || SettingOrigin {
        source: SettingSource::Default,
        reference: None,
    };
    for key in USER_FACING {
        provenance
            .0
            .entry((*key).to_owned())
            .or_insert_with(default);
    }
    for name in endpoints.keys() {
        for field in ["provider", "model", "base_url", "api_key"] {
            provenance
                .0
                .entry(format!("ai.endpoints[{name:?}].{field}"))
                .or_insert_with(default);
        }
    }
}

pub(crate) fn apply_coding_defaults(provenance: &mut SettingProvenance) {
    for key in CODING {
        provenance
            .0
            .entry((*key).to_owned())
            .or_insert(SettingOrigin {
                source: SettingSource::Default,
                reference: None,
            });
    }
}
