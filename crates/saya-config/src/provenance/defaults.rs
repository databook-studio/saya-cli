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
    "ai.app_attribution",
    "ai.temperature",
    "ai.timeout_seconds",
    "ai.idle_timeout_seconds",
    "ai.request_timeout_seconds",
    "ai.max_output_tokens",
    "ai.reasoning_effort",
    "ai.retry_delays_ms",
    "ai.context_byte_budget",
    "ai.context_window_tokens",
    "ai.show_thinking",
    "ai.compaction",
    "ai.compaction_trigger_percent",
    "run.read_only",
    "run.max_rows",
    "run.candidates",
    "run.query_timeout_seconds",
    "limits.session.requests",
    "limits.session.tool_calls",
    "limits.session.elapsed_seconds",
    "limits.session.known_tokens",
    "limits.ask.requests",
    "limits.ask.tool_calls",
    "limits.ask.elapsed_seconds",
    "limits.ask.known_tokens",
    "limits.run.wall_clock_seconds",
    "limits.run.tokens_per_endpoint",
    "limits.run.turns",
    "limits.run.tool_calls",
    "limits.run.no_progress_turns",
    "limits.run.repeat_limit",
    "jobs.fetch",
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
    "tools.allow",
    "permissions.allow_bypass",
];

/// Job defaults use the same value-free provenance fill as other resolved
/// settings: the `[job]` table's own keys, then the ceilings and the budget
/// mode `[limits.job]` carries.
const JOB: &[&str] = &[
    "job.automatic_lsp_profile",
    "job.program_dir",
    "job.root_mode",
    "job.extensions",
    "limits.job.budget_mode",
    "limits.job.logical_requests",
    "limits.job.transport_attempts",
    "limits.job.tool_calls",
    "limits.job.known_tokens",
    "limits.job.job_tokens",
    "limits.job.elapsed_seconds",
    "limits.job.no_progress_turns",
    "limits.job.repeat_limit",
    "limits.job.wall_clock_seconds",
    "limits.job.max_cost_usd",
];

/// The `[agents]` settings and the registry, on the same value-free fill.
const AGENTS: &[&str] = &[
    "agents.max_children",
    "agents.max_concurrent_model_requests",
    "agents",
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

pub(crate) fn apply_job_defaults(provenance: &mut SettingProvenance) {
    for key in JOB {
        provenance
            .0
            .entry((*key).to_owned())
            .or_insert(SettingOrigin {
                source: SettingSource::Default,
                reference: None,
            });
    }
}

/// The `[agents]` settings and the type registry, on the same value-free fill.
pub(crate) fn apply_agents_defaults(provenance: &mut SettingProvenance) {
    for key in AGENTS {
        provenance
            .0
            .entry((*key).to_owned())
            .or_insert(SettingOrigin {
                source: SettingSource::Default,
                reference: None,
            });
    }
}

/// `[tools]` has one key, and its default is "stated by nobody": an unstated
/// list narrows nothing, which is what every surface did before it existed.
const TOOLS: &[&str] = &["tools.allow"];

pub(crate) fn apply_tools_defaults(provenance: &mut SettingProvenance) {
    for key in TOOLS {
        provenance
            .0
            .entry((*key).to_owned())
            .or_insert(SettingOrigin {
                source: SettingSource::Default,
                reference: None,
            });
    }
}
