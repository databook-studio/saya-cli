//! Tests for the `workspace_write` tool: the first model-facing tool that
//! writes anything. The defining invariants: the loop's fail-closed permit
//! gate denies the call before the executor is reached (asserted on the
//! filesystem, not just the error), a permitted call lands exactly the bytes
//! given atomically, and every containment refusal surfaces the harness's
//! own reason while leaving the world outside the workspace untouched.

use std::{fs, path::PathBuf, sync::Arc};

use super::*;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentLimits, AgentRequest, AllowReadOnlyApproval, ChatProvider,
    ChatRequest, ChatResponse, LocalStateEffect, ToolCall, ToolDefinition, ToolError, ToolExecutor,
};
use saya_harness::workspace::Workspace;

use super::database_tools::WORKSPACE_WRITE_MAX_BYTES;

/// A sandbox workspace under the OS temp dir, removed on drop. The root sits
/// one level down (`outer/ws`) so a `../` escape has a real target
/// (`outer/outside.txt`) to name, and a symlink has a real victim to point at.
struct Sandbox {
    outer: PathBuf,
    ws: Workspace,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let outer =
            std::env::temp_dir().join(format!("saya-wswrite-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&outer);
        fs::create_dir_all(outer.join("ws")).expect("sandbox directory must create");
        let ws = Workspace::open(&outer.join("ws")).expect("workspace root must open");
        Self { outer, ws }
    }

    fn ws_root(&self) -> PathBuf {
        self.outer.join("ws")
    }

    /// Database tools with the sandbox workspace attached and no connections —
    /// a workspace-only run has no selected profile.
    fn tools(&self) -> DatabaseTools {
        DatabaseTools::with_registry(
            crate::connection::ConnectionRegistry::new("primary"),
            100,
            true,
            None,
        )
        .with_workspace(Some(Arc::new(self.ws.clone())))
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.outer);
    }
}

/// The real definition, taken from the advertised list: the loop tests must
/// exercise the gate against the definition the model would actually see.
fn workspace_write_definition() -> ToolDefinition {
    DatabaseTools::definitions(false, false, false, true, false)
        .into_iter()
        .find(|tool| tool.name == "workspace_write")
        .expect("workspace_write is advertised when workspace writes are permitted")
}

fn write_call(path: &str, content: &str) -> ToolCall {
    ToolCall {
        id: "c1".into(),
        name: "workspace_write".into(),
        arguments: serde_json::json!({"path": path, "content": content}),
    }
}

/// A provider that emits one tool call on its first `stream` invocation and a
/// plain text answer afterwards, so the run completes instead of looping on
/// the turn limit. Mirrors the harness in `saya-agent`'s local-state tests.
struct OneCallProvider {
    call: ToolCall,
    turn: std::sync::Mutex<u32>,
}

#[async_trait::async_trait]
impl ChatProvider for OneCallProvider {
    fn name(&self) -> &str {
        "one-call-mock"
    }
    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, saya_agent::ProviderError> {
        unreachable!("stream path is used")
    }
    async fn stream(
        &self,
        _: ChatRequest,
        _: saya_agent::CancellationToken,
    ) -> Result<saya_agent::ProviderStream, saya_agent::ProviderError> {
        let first = {
            let mut turn = self.turn.lock().unwrap();
            let was = *turn;
            *turn += 1;
            was == 0
        };
        let events = if first {
            vec![
                Ok(saya_agent::ProviderEvent::ToolCalls(vec![
                    self.call.clone(),
                ])),
                Ok(saya_agent::ProviderEvent::Done),
            ]
        } else {
            vec![
                Ok(saya_agent::ProviderEvent::TextDelta("done".into())),
                Ok(saya_agent::ProviderEvent::Done),
            ]
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

struct RecordingSink {
    events: std::sync::Arc<std::sync::Mutex<Vec<AgentEvent>>>,
}

#[async_trait::async_trait]
impl AgentEventSink for RecordingSink {
    async fn emit(&self, event: AgentEvent) {
        self.events.lock().unwrap().push(event);
    }
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "write a file".into(),
        profile_names: Vec::new(),
        model: "mock-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

include!("workspace_write_tests/basic_effects.rs");

include!("workspace_write_tests/path_and_size_safety.rs");

// -- create-if-absent and digest-guarded replacement (E03) --------------------

include!("workspace_write_tests/digest_create_and_replace.rs");

include!("workspace_write_tests/digest_refusal_and_race.rs");

include!("workspace_write_tests/definition_and_hygiene.rs");
