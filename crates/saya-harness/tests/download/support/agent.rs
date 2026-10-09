use super::transport::TestNet;
use super::*;

// --- the permit gate: the default denies http_download, no file appears -----

/// A provider that emits one tool call on its first `stream` invocation and
/// a plain answer afterwards, so a surviving turn completes.
pub(crate) struct OneCallProvider {
    pub(crate) call: ToolCall,
    pub(crate) turn: Mutex<u32>,
}

#[async_trait]
impl ChatProvider for OneCallProvider {
    fn name(&self) -> &str {
        "download-mock"
    }
    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("stream path is used")
    }
    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        let first = {
            let mut turn = self.turn.lock().unwrap();
            let was = *turn;
            *turn += 1;
            was == 0
        };
        let events = if first {
            vec![
                Ok(ProviderEvent::ToolCalls(vec![self.call.clone()])),
                Ok(ProviderEvent::Done),
            ]
        } else {
            vec![
                Ok(ProviderEvent::TextDelta("done".into())),
                Ok(ProviderEvent::Done),
            ]
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

/// An executor that really would download: if the loop ever ran it, the
/// workspace would gain the part file, the sidecar, and the destination.
pub(crate) struct DownloadExecutor {
    pub(crate) policy: FetchPolicy,
    pub(crate) net: TestNet,
    pub(crate) workspace: Workspace,
    pub(crate) calls: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl ToolExecutor for DownloadExecutor {
    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        self.calls.lock().unwrap().push(name.to_owned());
        let result = match http_download(
            &self.policy,
            &self.net,
            &self.workspace,
            arguments["destination"].as_str().unwrap_or_default(),
            arguments["url"].as_str().unwrap_or_default(),
            DownloadLimits::default(),
            &DownloadBudget::default(),
        )
        .await
        {
            Ok(outcome) => serde_json::json!({
                "destination": outcome.destination,
                "bytes": outcome.bytes,
                "sha256": outcome.sha256,
            }),
            Err(error) => serde_json::json!({"error": error.to_string()}),
        };
        Ok(result)
    }
}

pub(crate) struct RecordingSink {
    pub(crate) events: Arc<Mutex<Vec<AgentEvent>>>,
}

#[async_trait]
impl AgentEventSink for RecordingSink {
    async fn emit(&self, event: AgentEvent) {
        self.events.lock().unwrap().push(event);
    }
}

/// Approval granted to everything — so in the gate-isolation test only the
/// workspace permit gate can refuse the tool.
pub(crate) struct AllowApproval;

#[async_trait]
impl saya_agent::ApprovalDecider for AllowApproval {
    async fn approve(&self, _: &saya_agent::ToolDefinition, _: &serde_json::Value) -> bool {
        true
    }
}

pub(crate) fn agent_request() -> AgentRequest {
    AgentRequest {
        prompt: "fetch the corpus".into(),
        profile_names: vec!["analytics".into()],
        model: "mock-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}
