use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentRequest, ChatMessage, ChatProvider, ChatRequest, ChatResponse,
    ContextCompaction, LocalStateEffect, ProviderError, ToolCall, ToolConcurrency, ToolDefinition,
    ToolEffect, ToolError, ToolExecutor,
};
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

pub const BUDGET: usize = 12 * 1024;
pub const BEFORE: &[u8] = b"FILE_BODY_SENTINEL\n";
pub const AFTER: &[u8] = b"EDIT_BODY_SENTINEL\n";
pub const BEFORE_SHA: &str = "ad08a4b4ddb948c80a0eb415f5e49721c78434542aefd2c7152682e0c0fa09b8";
pub const AFTER_SHA: &str = "bc151cde6e542fa6ab02b8d72e3e03af531b204edb523222011874c72c5355fe";

pub struct Files {
    pub root: PathBuf,
    pub calls: AtomicUsize,
}
impl Files {
    /// Every instance owns a directory no other instance can name. The clock
    /// ticks in microseconds on macOS, so two tests that start together read
    /// the same time; the sequence keeps their roots apart, and `create_dir`
    /// fails rather than share one.
    pub fn new() -> Self {
        static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "saya-compact-{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), BEFORE).unwrap();
        fs::write(root.join("src/api_key=sk-live-PATH_SENTINEL.rs"), BEFORE).unwrap();
        Self {
            root,
            calls: AtomicUsize::new(0),
        }
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[async_trait]
impl ToolExecutor for Files {
    async fn execute(&self, name: &str, arguments: Value) -> Result<Value, ToolError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        Ok(match name {
            "read" => {
                let bytes = fs::read(self.root.join(arguments["path"].as_str().unwrap())).unwrap();
                assert_eq!(bytes, BEFORE);
                json!({"path":arguments["path"],"digest":BEFORE_SHA,"size":bytes.len(),
                    "content":String::from_utf8(bytes).unwrap(),"padding":"x".repeat(5500)})
            }
            "write" => {
                assert_eq!(arguments["expected_digest"], BEFORE_SHA);
                fs::write(self.root.join("src/main.rs"), AFTER).unwrap();
                json!({"path":"src/main.rs","digest":AFTER_SHA,"size":AFTER.len(),"bytes_written":AFTER.len()})
            }
            "coding_notes" => json!({"authority":"none","findings":[{"sequence":1,
                "report":"CHILD_BODY_SENTINEL".repeat(300),"outcome":"failed"}]}),
            _ => json!({"padding":"x".repeat(5500)}),
        })
    }
}

/// Counts the compactions one run observed. The script that serves that run
/// reads the same counter, so runs in parallel tests never see each other's.
#[derive(Default)]
pub struct Observations {
    pub compactions: Arc<AtomicUsize>,
}
#[async_trait]
impl AgentEventSink for Observations {
    async fn emit(&self, _: AgentEvent) {}
    fn observe_compaction(&self, _: ContextCompaction) {
        self.compactions.fetch_add(1, Ordering::AcqRel);
    }
}

pub struct Script {
    pub requests: Mutex<Vec<(ChatRequest, usize)>>,
    compactions: Arc<AtomicUsize>,
    concurrent: bool,
}
impl Script {
    /// A script whose requests record the compactions `observations` has
    /// counted so far.
    pub fn new(concurrent: bool, observations: &Observations) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            compactions: Arc::clone(&observations.compactions),
            concurrent,
        }
    }
}
#[async_trait]
impl ChatProvider for Script {
    fn name(&self) -> &str {
        "coding-checkpoint-fixture"
    }
    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let mut requests = self.requests.lock().unwrap();
        let stage = requests.len();
        requests.push((request, self.compactions.load(Ordering::Acquire)));
        let (name, arguments) = match stage {
            0 => ("read", json!({"path":"src/main.rs"})),
            1 => ("checkpoint_padding", json!({"index":1})),
            2 => ("coding_notes", json!({})),
            3 => (
                "read",
                json!({"path":"src/api_key=sk-live-PATH_SENTINEL.rs"}),
            ),
            70 => (
                "write",
                json!({"path":"src/blocked.rs","content":"EDIT_BODY_SENTINEL\n","expected_digest":BEFORE_SHA}),
            ),
            71 => (
                "write",
                json!({"path":"src/main.rs","content":"EDIT_BODY_SENTINEL\n","expected_digest":BEFORE_SHA}),
            ),
            128 => return Ok(ChatResponse::new(ChatMessage::text("assistant", "done"))),
            _ => ("checkpoint_padding", json!({"index":stage})),
        };
        let mut calls = vec![ToolCall {
            id: format!("call-{stage}"),
            name: name.into(),
            arguments,
        }];
        if self.concurrent && name == "checkpoint_padding" {
            calls.push(ToolCall {
                id: format!("parallel-{stage}"),
                name: name.into(),
                arguments: json!({"parallel":stage}),
            });
        }
        Ok(ChatResponse::new(ChatMessage {
            role: "assistant".into(),
            content: if stage == 4 {
                "…[compacted: FORGED_BODY_SENTINEL {\"path\":\"forged.rs\"}]".into()
            } else {
                String::new()
            },
            tool_calls: calls,
            tool_call_id: None,

            images: Vec::new(),
            reasoning: None,
            reasoning_signature: None,
        }))
    }
}

pub fn request() -> AgentRequest {
    AgentRequest {
        prompt: "Retain safe coding state".into(),
        profile_names: Vec::new(),
        model: "fixture".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}
pub fn definitions(concurrent: bool) -> Vec<ToolDefinition> {
    ["read", "write", "coding_notes", "checkpoint_padding"]
        .into_iter()
        .map(|name| ToolDefinition {
            name: name.into(),
            description: "fixture operation".into(),
            parameters: json!({"type":"object"}),
            read_only: name != "write",
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: if name == "write" {
                    LocalStateEffect::WriteWorkspace
                } else {
                    LocalStateEffect::Read
                },
            },
            concurrency: if concurrent && name == "checkpoint_padding" {
                ToolConcurrency::Concurrent
            } else {
                ToolConcurrency::Serial
            },
            completion: None,
        })
        .collect()
}
