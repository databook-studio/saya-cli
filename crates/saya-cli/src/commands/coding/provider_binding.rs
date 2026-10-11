use super::{CodingApplication, CodingError, NativeCodingJob, create::digest};
use serde_json::{Value, json};
impl CodingApplication {
    pub(super) fn provider_bindings(&self) -> Result<Value, CodingError> {
        let root = &self.runtime.resolved.ai;
        let mut child = root.clone();
        let endpoint = self
            .runtime
            .resolved
            .endpoints
            .get(saya_config::ORCHESTRATOR_ROLE)
            .ok_or(CodingError::InvalidRequest)?;
        child.provider = endpoint.provider;
        child.model = endpoint.model.clone();
        child.base_url = endpoint.base_url.clone();
        child.api_key = endpoint.api_key.clone();
        Ok(json!({"root":binding(root)?,"child":binding(&child)?}))
    }
}
fn binding(ai: &saya_config::ResolvedAi) -> Result<Value, CodingError> {
    let endpoint = match ai.base_url.as_deref() {
        Some(value) => {
            let mut url = url::Url::parse(value).map_err(CodingError::owner)?;
            url.set_username("")
                .map_err(|_| CodingError::InvalidRequest)?;
            url.set_password(None)
                .map_err(|_| CodingError::InvalidRequest)?;
            url.set_query(None);
            url.set_fragment(None);
            url.to_string()
        }
        None => format!("{}:provider-default", ai.provider.as_str()),
    };
    Ok(
        json!({"provider":ai.provider.as_str(),"model_id":saya_types::redact(&ai.model),"endpoint_fingerprint":digest(endpoint.as_bytes())?,
        "max_output_tokens":ai.max_output_tokens,"temperature":ai.temperature,"timeout_seconds":ai.timeout_seconds,"idle_timeout_seconds":ai.idle_timeout_seconds,"retry_delays_ms":ai.retry_delays_ms,
        "declared_context_window_tokens":ai.context_window_tokens,"context_bound_verified":false,"source":"actual_native_provider_settings_projection"}),
    )
}
impl NativeCodingJob {
    pub(super) fn record_provider_binding(&self) -> Result<(), CodingError> {
        let value = json!({"epoch":self.epoch,"job_id":self.job_id(),"policy_sha256":self.policy_digest,"bindings":self.application.provider_bindings()?});
        super::context::append_line(
            &self.records.join("provider-bindings.jsonl"),
            &serde_json::to_vec(&value).map_err(CodingError::owner)?,
            1024 * 1024,
        )
    }
}
