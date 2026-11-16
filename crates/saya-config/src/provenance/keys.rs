//! The diagnostics-field → dotted-key table backing `config show --explain`.

/// The `config show` diagnostics fields, each paired with the dotted config
/// key the merge funnels record it under, in the order `--explain` prints.
/// `profile_dialect` has no entry (derived from the selected profile, not a
/// setting) and the `endpoints` map has none (the recorder keys each
/// endpoint's fields individually), so they stay out of the explanation.
const USER_FACING_DIAGNOSTICS: &[(&str, &str)] = &[
    ("profile_name", "profile_name"),
    ("provider", "ai.provider"),
    ("model", "ai.model"),
    ("base_url", "ai.base_url"),
    ("api_key_reference", "ai.api_key"),
    ("allow_data_sharing", "ai.allow_data_sharing"),
    ("app_attribution", "ai.app_attribution"),
    ("temperature", "ai.temperature"),
    ("timeout_seconds", "ai.timeout_seconds"),
    ("idle_timeout_seconds", "ai.idle_timeout_seconds"),
    ("request_timeout_seconds", "ai.request_timeout_seconds"),
    ("max_output_tokens", "ai.max_output_tokens"),
    ("reasoning_effort", "ai.reasoning_effort"),
    ("retry_delays_ms", "ai.retry_delays_ms"),
    ("context_byte_budget", "ai.context_byte_budget"),
    ("context_window_tokens", "ai.context_window_tokens"),
    ("show_thinking", "ai.show_thinking"),
    ("compaction", "ai.compaction"),
    (
        "compaction_trigger_percent",
        "ai.compaction_trigger_percent",
    ),
    ("read_only", "run.read_only"),
    ("max_rows", "run.max_rows"),
    ("candidates", "run.candidates"),
    ("limits_session_requests", "limits.session.requests"),
    ("limits_session_tool_calls", "limits.session.tool_calls"),
    (
        "limits_session_elapsed_seconds",
        "limits.session.elapsed_seconds",
    ),
    ("limits_session_known_tokens", "limits.session.known_tokens"),
    ("limits_ask_requests", "limits.ask.requests"),
    ("limits_ask_tool_calls", "limits.ask.tool_calls"),
    ("limits_ask_elapsed_seconds", "limits.ask.elapsed_seconds"),
    ("limits_ask_known_tokens", "limits.ask.known_tokens"),
    ("limits_job_budget_mode", "limits.job.budget_mode"),
    ("agents_max_children", "agents.max_children"),
    (
        "agents_max_concurrent_model_requests",
        "agents.max_concurrent_model_requests",
    ),
    ("job_program_dir", "job.program_dir"),
    ("job_root_mode", "job.root_mode"),
    ("limits_job_logical_requests", "limits.job.logical_requests"),
    (
        "limits_job_transport_attempts",
        "limits.job.transport_attempts",
    ),
    ("limits_job_tool_calls", "limits.job.tool_calls"),
    ("limits_job_known_tokens", "limits.job.known_tokens"),
    ("limits_job_job_tokens", "limits.job.job_tokens"),
    ("limits_job_elapsed_seconds", "limits.job.elapsed_seconds"),
    (
        "limits_job_no_progress_turns",
        "limits.job.no_progress_turns",
    ),
    ("limits_job_repeat_limit", "limits.job.repeat_limit"),
    (
        "limits_job_wall_clock_seconds",
        "limits.job.wall_clock_seconds",
    ),
    ("limits_job_max_cost_usd", "limits.job.max_cost_usd"),
    ("job_automatic_lsp_profile", "job.automatic_lsp_profile"),
    ("job_extensions", "job.extensions"),
    ("agents", "agents"),
    ("query_timeout_seconds", "run.query_timeout_seconds"),
    ("tools_allow", "tools.allow"),
    ("permissions_allow_bypass", "permissions.allow_bypass"),
    (
        "limits_run_wall_clock_seconds",
        "limits.run.wall_clock_seconds",
    ),
    (
        "limits_run_tokens_per_endpoint",
        "limits.run.tokens_per_endpoint",
    ),
    ("limits_run_turns", "limits.run.turns"),
    ("limits_run_tool_calls", "limits.run.tool_calls"),
    (
        "limits_run_no_progress_turns",
        "limits.run.no_progress_turns",
    ),
    ("limits_run_repeat_limit", "limits.run.repeat_limit"),
    ("output_format", "output.format"),
    ("output_color", "output.color"),
    ("ui_theme", "ui.theme"),
    ("memory_mode", "memory.mode"),
    ("memory_max_contracts", "memory.max_contracts"),
    (
        "memory_max_claims_per_contract",
        "memory.max_claims_per_contract",
    ),
    ("memory_max_context_bytes", "memory.max_context_bytes"),
];

/// The diagnostics-field → dotted-key table `config show --explain` renders:
/// which `ResolvedDiagnostics` field each user-facing setting rides on, and
/// the key the merge funnels record it under.
pub fn user_facing_keys() -> &'static [(&'static str, &'static str)] {
    USER_FACING_DIAGNOSTICS
}
