use super::{CodingApplication, CodingError};
use async_trait::async_trait;
use saya_agent::{
    AgentError, AgentLimits, AgentRequest, AgentRunControl, AllowReadOnlyApproval, AnsweringLimits,
    CancellationToken, NoopEventSink, ProviderError, ToolError, ToolExecutor,
    run_agent_with_control,
};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};

/// The bounded probe's outcome. `completed` keeps the boolean the doctor has
/// always reported; `truncated_by_output_limit` records that the run was cut
/// off by the provider's output-token limit, so the doctor can say so without
/// changing any durable format.
#[derive(Clone, Copy)]
pub(super) struct ProbeOutcome {
    pub(super) completed: bool,
    pub(super) truncated_by_output_limit: bool,
}
struct Probe {
    called: AtomicBool,
}
#[async_trait]
impl ToolExecutor for Probe {
    async fn execute(&self, name: &str, arguments: Value) -> Result<Value, ToolError> {
        if name != "coding_readiness_probe" || arguments != json!({"check":"native_tools"}) {
            return Err(ToolError::UnsupportedTool);
        }
        self.called.store(true, Ordering::Release);
        Ok(json!({"native_tools":"available"}))
    }
}
impl CodingApplication {
    /// Explicit opt-in bounded provider call. The probe owns no database,
    /// workspace or process tool, and receives no repository content.
    pub(super) async fn probe_provider(&self) -> Result<ProbeOutcome, CodingError> {
        let mut ai = self.runtime.resolved.ai.clone();
        ai.timeout_seconds = 10;
        ai.idle_timeout_seconds = 10;
        ai.retry_delays_ms.clear();
        let provider = crate::agent::provider::build(&ai, &self.runtime.secret_resolver())
            .map_err(CodingError::owner)?;
        let probe = Probe {
            called: AtomicBool::new(false),
        };
        let definition = crate::agent::tools::coding::definition(
            "coding_readiness_probe",
            "Verify the native tool-call loop without external effects",
            json!({"type":"object","properties":{"check":{"type":"string","enum":["native_tools"]}},"required":["check"],"additionalProperties":false}),
            false,
        );
        let token = CancellationToken::new();
        let limits = AgentLimits {
            max_turns: Some(2),
            max_tool_calls: Some(1),
            max_continuations: Some(0),
            context_byte_budget: 8192,
            ..AgentLimits::default()
        };
        let request = AgentRequest {
            prompt:
                "Call coding_readiness_probe with check native_tools once, then report readiness."
                    .into(),
            profile_names: vec![],
            model: ai.model,
            system_prompt: None,
            history: vec![],
            context_blocks: vec![],
        };
        let result = run_agent_with_control(
            provider.as_ref(),
            &probe,
            request,
            vec![definition],
            AgentRunControl::new(
                limits,
                AnsweringLimits {
                    max_requests: Some(2),
                    max_tool_calls: Some(1),
                    max_reported_tokens: Some(4096),
                },
                &AllowReadOnlyApproval,
                &NoopEventSink,
                token,
            ),
        )
        .await;
        let completed = result.is_ok() && probe.called.load(Ordering::Acquire);
        let truncated_by_output_limit = matches!(
            result,
            Err(AgentError::Provider(ProviderError::OutputTruncated { .. }))
        );
        Ok(ProbeOutcome {
            completed,
            truncated_by_output_limit,
        })
    }
}
