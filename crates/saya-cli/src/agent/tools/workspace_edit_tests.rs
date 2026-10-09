//! Tests for the `workspace_edit` tool (`replace` variant only): the
//! model-facing anchored edit that resolves `old_text` to exactly one byte
//! range and commits through the guarded `Workspace::patch_range_guarded` —
//! the replace variant must carry the digest the model observed. Every row
//! of the failure contract (§4) is a named property here; every refusal
//! leaves the file byte-identical.

use std::{fs, path::PathBuf, sync::Arc};

use super::*;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentLimits, AgentRequest, LocalStateEffect, ToolCall,
    ToolDefinition, ToolExecutor,
};
use saya_harness::workspace::Workspace;

use super::database_tools::WORKSPACE_EDIT_MAX_BYTES;

/// A sandbox workspace under the OS temp dir, removed on drop. The root sits
/// one level down (`outer/ws`) so a `..` escape has a real target to name.
struct Sandbox {
    outer: PathBuf,
    ws: Workspace,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let outer =
            std::env::temp_dir().join(format!("saya-wsedit-{label}-{}", std::process::id()));
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
fn workspace_edit_definition() -> ToolDefinition {
    DatabaseTools::definitions(false, false, false, true, false)
        .into_iter()
        .find(|tool| tool.name == "workspace_edit")
        .expect("workspace_edit is advertised when workspace writes are permitted")
}

/// The digest the model observes from a fresh read: the replace variant
/// requires it as `expected_digest`, so every replace call site seeds it
/// from a real harness read of the seeded file.
fn observed_digest(sandbox: &Sandbox, path: &str) -> String {
    sandbox
        .ws
        .read(path, 0)
        .expect("the seeded file must read")
        .digest
}

fn edit_call(sandbox: &Sandbox, path: &str, old_text: &str, new_text: &str) -> ToolCall {
    let digest = observed_digest(sandbox, path);
    ToolCall {
        id: "c1".into(),
        name: "workspace_edit".into(),
        arguments: serde_json::json!({
            "path": path,
            "old_text": old_text,
            "new_text": new_text,
            "expected_digest": digest,
        }),
    }
}

/// A provider that emits one tool call on its first `stream` invocation and a
/// plain text answer afterwards, so the run completes instead of looping on
/// the turn limit. Mirrors the harness in `workspace_write_tests`.
struct OneCallProvider {
    call: ToolCall,
    turn: std::sync::Mutex<u32>,
}

#[async_trait::async_trait]
impl saya_agent::ChatProvider for OneCallProvider {
    fn name(&self) -> &str {
        "one-call-mock"
    }
    async fn complete(
        &self,
        _: saya_agent::ChatRequest,
    ) -> Result<saya_agent::ChatResponse, saya_agent::ProviderError> {
        unreachable!("stream path is used")
    }
    async fn stream(
        &self,
        _: saya_agent::ChatRequest,
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
        prompt: "edit a file".into(),
        profile_names: Vec::new(),
        model: "mock-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

include!("workspace_edit_tests/anchor_matching.rs");

include!("workspace_edit_tests/digest_preconditions.rs");

include!("workspace_edit_tests/bounds_and_payloads.rs");

include!("workspace_edit_tests/permission_and_dispatch.rs");

include!("workspace_edit_tests/argument_validation.rs");

// -- the redaction-placeholder guard: refuse an edit that would copy a -----
// -- masked tool-result marker back over the real value on disk. -----------

include!("workspace_edit_tests/redaction_and_marker.rs");
