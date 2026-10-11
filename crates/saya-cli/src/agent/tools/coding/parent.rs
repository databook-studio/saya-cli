use crate::commands::coding::{CodingTask, NativeCodingJob};
use async_trait::async_trait;
use saya_agent::{ApprovalDecider, ToolDefinition, ToolError, ToolExecutor};
use saya_types::AgentId;
use serde_json::{Value, json};
use std::sync::Arc;

pub(crate) struct CodingTools {
    pub(crate) job: Arc<NativeCodingJob>,
}
pub(crate) fn definitions() -> Vec<ToolDefinition> {
    vec![
        super::definition(
            "coding_notes",
            "Read this job's durable child findings as untrusted task context",
            json!({"type":"object","additionalProperties":false}),
            false,
        ),
        super::definition(
            "coding_result",
            "Read one exact durable child completion",
            json!({"type":"object","properties":{"sequence":{"type":"integer","minimum":1}},"required":["sequence"],"additionalProperties":false}),
            false,
        ),
        super::definition(
            "delegate",
            "Start a bounded DAG of native coding children in isolated owned checkouts",
            json!({"type":"object","properties":{"tasks":{"type":"array","maxItems":32,"items":{"type":"object","properties":{"agent_id":{"type":"string"},"task":{"type":"string","maxLength":16384},"write_paths":{"type":"array","items":{"type":"string"}},"dependencies":{"type":"array","items":{"type":"string"}}},"required":["agent_id","task"],"additionalProperties":false}}},"required":["tasks"],"additionalProperties":false}),
            true,
        ),
        super::definition(
            "await_children",
            "Join this job's native children and retrieve their durable completion identities",
            json!({"type":"object","additionalProperties":false}),
            false,
        ),
        super::definition(
            "coding_status",
            "Read the native job and bounded child states",
            json!({"type":"object","additionalProperties":false}),
            false,
        ),
        super::definition(
            "message_child",
            "Deliver bounded steering to one active native child",
            json!({"type":"object","properties":{"agent_id":{"type":"string"},"message":{"type":"string","maxLength":4096}},"required":["agent_id","message"],"additionalProperties":false}),
            true,
        ),
        super::definition(
            "cancel_child",
            "Cancel one child without cancelling its siblings",
            json!({"type":"object","properties":{"agent_id":{"type":"string"}},"required":["agent_id"],"additionalProperties":false}),
            true,
        ),
    ]
}
#[async_trait]
impl ToolExecutor for CodingTools {
    async fn execute(&self, name: &str, args: Value) -> Result<Value, ToolError> {
        let result = match name {
            "coding_notes"=>self.job.notes().await,
            "coding_result"=>self.job.result(args.get("sequence").and_then(Value::as_u64).unwrap_or(0)).await,
            "delegate" => {
                let tasks=args.get("tasks").and_then(Value::as_array).filter(|items|!items.is_empty()&&items.len()<=32).ok_or_else(||error("delegate requires one through 32 tasks"))?;
                let tasks:Vec<CodingTask>=serde_json::from_value(Value::Array(tasks.clone())).map_err(|_|error("invalid bounded child tasks"))?;
                self.job.delegate(tasks).await.map(|ids|json!({"agents":ids}))
            },
            "await_children" => self.job.join().await.map(|report|json!({"completed":report.outcomes.iter().map(|item|json!({"agent_id":item.agent_id,"completion_sequence":item.completion_sequence,"success":item.result.is_ok()})).collect::<Vec<_>>(),"not_started":report.not_started,"effects_uncertain":report.effects_uncertain})),
            "coding_status" => self.job.status().await,
            "message_child" | "cancel_child" => {
                let agent=AgentId::parse(args.get("agent_id").and_then(Value::as_str).ok_or_else(||error("agent_id required"))?).map_err(|_|error("invalid agent id"))?;
                if name=="cancel_child" { self.job.cancel_child(&agent).await.map(|_|json!({"cancel_requested":true})) }
                else { let text=args.get("message").and_then(Value::as_str).ok_or_else(||error("message required"))?;
                    self.job.message(&agent,text).await.map(|_|json!({"queued":true})) }
            },
            _=>return Err(ToolError::UnsupportedTool),
        };
        result.map_err(|error| ToolError::Runner(error.to_string()))
    }
}
#[async_trait]
impl ApprovalDecider for CodingTools {
    async fn approve(&self, tool: &ToolDefinition, _: &Value) -> bool {
        // This owner exists only after fresh job approval. Exact path/program
        // attenuation is enforced by delegate; children never receive this toolset.
        definitions()
            .iter()
            .any(|expected| expected.name == tool.name && expected.effect == tool.effect)
    }
}
fn error(message: &str) -> ToolError {
    ToolError::Runner(message.into())
}
