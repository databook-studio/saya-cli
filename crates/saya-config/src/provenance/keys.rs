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
    ("temperature", "ai.temperature"),
    ("timeout_seconds", "ai.timeout_seconds"),
    ("idle_timeout_seconds", "ai.idle_timeout_seconds"),
    ("max_output_tokens", "ai.max_output_tokens"),
    ("retry_delays_ms", "ai.retry_delays_ms"),
    ("context_byte_budget", "ai.context_byte_budget"),
    ("context_window_tokens", "ai.context_window_tokens"),
    ("show_thinking", "ai.show_thinking"),
    ("compaction", "ai.compaction"),
    ("read_only", "run.read_only"),
    ("max_rows", "run.max_rows"),
    ("max_iterations", "run.max_iterations"),
    ("candidates", "run.candidates"),
    ("coding_enabled", "coding.enabled"),
    ("coding_max_children", "coding.max_children"),
    (
        "coding_max_concurrent_model_requests",
        "coding.max_concurrent_model_requests",
    ),
    (
        "coding_max_concurrent_commands",
        "coding.max_concurrent_commands",
    ),
    (
        "coding_max_logical_model_requests",
        "coding.max_logical_model_requests",
    ),
    (
        "coding_max_physical_transport_attempts",
        "coding.max_physical_transport_attempts",
    ),
    ("coding_max_tool_calls", "coding.max_tool_calls"),
    ("coding_max_known_tokens", "coding.max_known_tokens"),
    ("coding_max_elapsed_seconds", "coding.max_elapsed_seconds"),
    ("coding_check_profiles", "coding.check_profiles"),
    ("query_timeout_seconds", "run.query_timeout_seconds"),
    ("jobs_wall_clock_seconds", "jobs.wall_clock_seconds"),
    ("jobs_tokens_per_endpoint", "jobs.tokens_per_endpoint"),
    ("jobs_turns", "jobs.turns"),
    ("jobs_tool_calls", "jobs.tool_calls"),
    ("jobs_runner_allow", "jobs.runner"),
    ("jobs_runner_program_dir", "jobs.runner"),
    ("jobs_runner_timeout_seconds", "jobs.runner"),
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
