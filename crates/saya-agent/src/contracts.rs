use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Provider-independent request passed to a future agent loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRequest {
    pub prompt: String,
    pub profile_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    TextDelta { text: String },
    ToolRequested { name: String },
    Complete,
}

#[async_trait]
pub trait EventSink: Send + Sync {
    async fn emit(&self, event: AgentEvent);
}

#[async_trait]
pub trait AiProvider: Send + Sync {
    fn name(&self) -> &str;
    fn is_cloud(&self) -> bool;
    async fn stream(&self, request: AgentRequest, sink: &dyn EventSink) -> Result<(), String>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub read_only: bool,
}

#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, String>;
}
