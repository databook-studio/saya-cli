use async_trait::async_trait;
use saya_agent::{
    AgentLimits, AgentRequest, ChatMessage, ChatProvider, ChatRequest, ChatResponse,
    LocalStateEffect, ProviderError, ToolCall, ToolConcurrency, ToolDefinition, ToolEffect,
    ToolError, ToolExecutor,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
mod owner;
pub use owner::Owner;

pub struct Files {
    pub root: PathBuf,
    pub reads: Mutex<BTreeMap<String, usize>>,
    pub writes: AtomicUsize,
    pub failure: &'static str,
    pub barrier: Option<tokio::sync::Barrier>,
}
impl Files {
    pub fn new(failure: &'static str) -> Self {
        static NEXT_ROOT: AtomicUsize = AtomicUsize::new(0);
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("saya-retry-{}-{sequence}", std::process::id()));
        fs::create_dir(&root).unwrap();
        Self {
            root,
            reads: Mutex::new(BTreeMap::new()),
            writes: AtomicUsize::new(0),
            failure,
            barrier: None,
        }
    }
    pub fn read_count(&self, path: &str) -> usize {
        *self.reads.lock().unwrap().get(path).unwrap_or(&0)
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[async_trait]
impl ToolExecutor for Files {
    async fn execute_with_context(
        &self,
        name: &str,
        arguments: Value,
        context: saya_agent::ToolExecutionContext,
    ) -> Result<Value, ToolError> {
        let result = self.execute(name, arguments).await;
        if self.failure == "cancel_write" && name == "write" {
            context.cancellation.cancel();
        }
        result
    }
    async fn execute(&self, name: &str, arguments: Value) -> Result<Value, ToolError> {
        let path = arguments["path"].as_str().unwrap();
        if name == "write" || name == "edit" {
            self.writes.fetch_add(1, Ordering::AcqRel);
            if self.failure == "write_refused" {
                return Err(ToolError::NoEffect(Box::new(ToolError::WorkspaceWrite(
                    "known refusal".into(),
                ))));
            }
            if self.failure == "overlap_mutation" {
                tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    self.barrier.as_ref().unwrap().wait(),
                )
                .await
                .expect("the actual mutator overlaps the read");
            }
            fs::write(self.root.join(path), b"created\n").unwrap();
            if self.failure == "write_unknown" {
                return Err(ToolError::WorkspaceWrite("ambiguous write".into()));
            }
            return Ok(json!({"path":path,"size":8,"digest":"d".repeat(64)}));
        }
        let count = {
            let mut reads = self.reads.lock().unwrap();
            let count = reads.entry(path.into()).or_default();
            *count += 1;
            *count
        };
        let snapshot = fs::read(self.root.join(path));
        if count == 1
            && let Some(barrier) = &self.barrier
        {
            tokio::time::timeout(std::time::Duration::from_secs(2), barrier.wait())
                .await
                .expect("the actual read-only batch must overlap");
        }
        match snapshot {
            Ok(bytes) => Ok(
                json!({"path":path,"content":String::from_utf8(bytes).unwrap(),"size":8,"digest":"d".repeat(64)}),
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => match self.failure {
                "opaque" => Err(ToolError::Workspace("file does not exist".into())),
                "opaque_wrapped" => Err(ToolError::NoEffect(Box::new(ToolError::Workspace(
                    "file does not exist".into(),
                )))),
                "deterministic" => Err(ToolError::Workspace("containment refusal".into())),
                "wrapped" => Err(ToolError::NoEffect(Box::new(
                    ToolError::WorkspaceFileMissing,
                ))),
                _ => Err(ToolError::WorkspaceFileMissing),
            },
            Err(error) => panic!("unexpected test filesystem failure: {error}"),
        }
    }
}

pub struct Script {
    pub turns: Mutex<Vec<ChatMessage>>,
    pub requests: Mutex<Vec<ChatRequest>>,
    pub remove_before_retry: Option<PathBuf>,
}
impl Script {
    pub fn new(turns: Vec<ChatMessage>) -> Self {
        Self {
            turns: Mutex::new(turns),
            requests: Mutex::new(Vec::new()),
            remove_before_retry: None,
        }
    }
    pub fn result(&self, id: &str) -> Value {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .flat_map(|request| &request.messages)
            .find(|message| message.tool_call_id.as_deref() == Some(id))
            .map(|message| serde_json::from_str(&message.content).unwrap())
            .expect("actual provider result")
    }
}
#[async_trait]
impl ChatProvider for Script {
    fn name(&self) -> &str {
        "workspace-state-fixture"
    }
    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let mut requests = self.requests.lock().unwrap();
        if requests.len() == 2
            && let Some(path) = &self.remove_before_retry
        {
            fs::remove_file(path).unwrap();
        }
        requests.push(request);
        Ok(ChatResponse::new(self.turns.lock().unwrap().remove(0)))
    }
}
pub fn call(id: &str, name: &str, path: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        arguments: json!({"path":path}),
    }
}
pub fn turn(calls: Vec<ToolCall>) -> ChatMessage {
    ChatMessage {
        role: "assistant".into(),
        content: String::new(),
        tool_calls: calls,
        tool_call_id: None,
        images: Vec::new(),
        reasoning: None,
        reasoning_signature: None,
    }
}
pub fn done() -> ChatMessage {
    ChatMessage::text("assistant", "done")
}
pub fn request() -> AgentRequest {
    AgentRequest {
        prompt: "recover a missing file safely".into(),
        profile_names: Vec::new(),
        model: "fixture".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}
pub fn limits() -> AgentLimits {
    AgentLimits {
        permit_workspace_writes: true,
        context_byte_budget: 64 * 1024,
        ..AgentLimits::default()
    }
}
pub fn definitions(concurrent: bool) -> Vec<ToolDefinition> {
    ["read", "write", "edit"]
        .into_iter()
        .map(|name| ToolDefinition {
            name: name.into(),
            description: "bounded fixture filesystem operation".into(),
            parameters: json!({"type":"object"}),
            read_only: name == "read",
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: if name == "read" {
                    LocalStateEffect::Read
                } else {
                    LocalStateEffect::WriteWorkspace
                },
            },
            concurrency: if concurrent && name == "read" {
                ToolConcurrency::Concurrent
            } else {
                ToolConcurrency::Serial
            },
            completion: None,
        })
        .collect()
}
