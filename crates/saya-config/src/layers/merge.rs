use super::*;

pub(crate) fn merge(
    base: &mut ConfigFile,
    layer: &ConfigFile,
    recorder: &mut Recorder,
    source: SettingSource,
) {
    macro_rules! apply { ($($path:ident).+) => { if layer.$($path).+.is_some() { base.$($path).+ = layer.$($path).+.clone(); recorder.record(stringify!($($path).+), source.clone()); } }; }
    apply!(default_profile);
    apply!(ai.provider);
    apply!(ai.model);
    apply!(ai.base_url);
    apply!(ai.allow_data_sharing);
    // The API key records the winning reference's redacted label, not just
    // the layer — the reference is the only display-safe fact about a secret.
    if let Some(secret) = layer.ai.api_key.as_ref() {
        base.ai.api_key = Some(secret.clone());
        recorder.record_secret("ai.api_key", source.clone(), secret);
    }
    apply!(ai.temperature);
    apply!(ai.timeout_seconds);
    apply!(ai.idle_timeout_seconds);
    apply!(ai.max_output_tokens);
    apply!(ai.retry_delays_ms);
    apply!(ai.context_byte_budget);
    apply!(ai.investigation_max_logical_answering_requests);
    apply!(ai.investigation_max_requested_tool_calls);
    apply!(ai.investigation_max_elapsed_seconds);
    apply!(ai.investigation_max_known_reported_tokens);
    apply!(ai.context_window_tokens);
    apply!(ai.show_thinking);
    apply!(ai.compaction);
    apply!(run.read_only);
    apply!(run.max_rows);
    apply!(run.max_iterations);
    apply!(run.candidates);
    apply!(run.query_timeout_seconds);
    apply!(jobs.wall_clock_seconds);
    apply!(jobs.tokens_per_endpoint);
    apply!(jobs.turns);
    apply!(jobs.tool_calls);
    apply!(jobs.fetch);
    apply!(jobs.runner);
    apply!(jobs.interpreter);
    // `[host_commands]` merges like any ordinary user-layer section here —
    // the project layer never reaches this merge with the section set (the
    // typed refusal in `resolve.rs` fires first).
    if !layer.host_commands.pass_env.is_empty() {
        base.host_commands.pass_env = layer.host_commands.pass_env.clone();
        recorder.record("host_commands.pass_env", source.clone());
    }
    if layer.host_commands.timeout_seconds.is_some() {
        base.host_commands.timeout_seconds = layer.host_commands.timeout_seconds;
        recorder.record("host_commands.timeout_seconds", source.clone());
    }
    // `[session_commands] deny` merges like any ordinary user-layer section
    // here — the project layer never reaches this merge with the section set
    // (the typed refusal in `resolve.rs` fires first).
    if !layer.session_commands.deny.is_empty() {
        base.session_commands.deny = layer.session_commands.deny.clone();
        recorder.record("session_commands.deny", source.clone());
    }
    apply!(output.format);
    apply!(output.color);
    apply!(ui.theme);
    apply!(memory.mode);
    apply!(memory.max_contracts);
    apply!(memory.max_claims_per_contract);
    apply!(memory.max_context_bytes);
    // `[instructions]` merges per field like the other scalar sections; the
    // trust boundary in `revert_untrusted` reverts all five keys, since they
    // decide what instruction context the model is assembled from.
    apply!(instructions.project);
    apply!(instructions.personal);
    apply!(instructions.project_max_bytes);
    apply!(instructions.personal_max_bytes);
    apply!(instructions.total_max_bytes);
    // `[[ai.endpoints]]` is a set of named entries, not a scalar: a layer's
    // entry for an existing name overlays that endpoint's fields (absent
    // field = leave the lower layer's value, matching `apply!`), a new name
    // is added. The trust boundary in `revert_untrusted` decides which of
    // those overlays are permitted.
    for endpoint in &layer.ai.endpoints {
        match base
            .ai
            .endpoints
            .iter_mut()
            .find(|existing| existing.name == endpoint.name)
        {
            Some(existing) => {
                if endpoint.provider.is_some() {
                    existing.provider = endpoint.provider;
                }
                if endpoint.model.is_some() {
                    existing.model = endpoint.model.clone();
                }
                if endpoint.base_url.is_some() {
                    existing.base_url = endpoint.base_url.clone();
                }
                if endpoint.api_key.is_some() {
                    existing.api_key = endpoint.api_key.clone();
                }
            }
            None => base.ai.endpoints.push(endpoint.clone()),
        }
        record_endpoint_fields(recorder, &endpoint.name, endpoint, source.clone());
    }
}
